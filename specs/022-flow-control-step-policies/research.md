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
- **D022-078 — The declaration a waiter is bounded by is a column of its own, `pinned_limit`, and the order bound of a
  pass starts at any refusal for lack of room (2026-11-24; decision D022-076 items 4 and 7).** Delegate
  (OD-2026-11-24-4). Building decision D022-076 found two places where its text cannot be built as written:
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
     `least(coalesce(pinned_limit, declared_limit), the current ceiling)`; a CHECK holds `pinned_limit >= declared_limit`
     (the filed limit is never above the declaration), added `NOT VALID` and validated by a second migration. The
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

### Decisions of the design of the rest of 022 for release 0.3.0 (2026-11-24)

The owner put the whole of feature 022 in release 0.3.0 (OD-2026-11-24-1, "Inclua tudo na 0.3.0") and delegated its open
questions, ADR acceptances and review findings to the coordinating session (OD-2026-11-24-4, "Tomem a melhor decisão").
The decisions below take the block D022-200 upward (other branches use the numbers below 200). Each is recorded as
"delegate (OD-2026-11-24-4)" with the options weighed and the reason; each was decided for the most architectural and
business advantage, never for simplicity alone. The design they lead to is in three proposed ADRs:
[ADR-0135](../../docs/adr/ADR-0135-a-step-cache-hit-is-a-journaled-completion-decided-before-the-gates.md) (the step result
cache), [ADR-0136](../../docs/adr/ADR-0136-failure-handlers-run-between-the-stop-and-the-terminal-commit.md) (failure
handlers and early return) and
[ADR-0137](../../docs/adr/ADR-0137-standalone-and-draft-test-runs-are-ordinary-runs-of-private-definitions.md) (standalone
steps and draft test runs). The coordinating session accepts or rejects them after the independent reviews (task
T022-200). The code facts below were checked on this branch at `d8d3cf79`.

- **D022-200 — Draft test runs are refused in `prod` namespaces; `dev` and `staging` allow them (O1160 option (a)).**
  Delegate (OD-2026-11-24-4). Options: (a) refuse in `prod` (`FORBIDDEN`, `TEST_RUN_PRODUCTION_NAMESPACE`), allow `dev`
  and `staging`; (b) allow in `prod` behind an opt-in permission and a per-run audit; (c) allow only in `dev`. Chosen
  (a). An unmocked step of a test run is a real execution with real effects, under the draft's code that nobody has
  published or reviewed; in `prod` that is an unreviewed deployment. Production debugging already has an audited,
  reviewed path: 018's reset of a production run to a point, which reuses the published version. (b) needs its own
  audit design, a second approval vocabulary and a way to stop a test run's effects that nobody has asked for; (c)
  takes away the `staging` rehearsal that customers use to test against production-shaped connections. The refusal is
  decided before any row exists, from `namespaces.environment` (migration `20261008000000_identity`).
- **D022-201 — A failure handler never changes the run's outcome; recovery is a handler that starts a reset or a rerun
  (O1161 option (a)).** Delegate (OD-2026-11-24-4). Options: (a) the handler never changes the outcome, and a
  handler that wants the work done again starts a reset (018) or a rerun as new, an audited new run with provenance;
  (b) Windmill's `recover: true`, which turns the failure into success; (c) a handler that may set a reduced status
  (`PARTIAL`). Chosen (a). After compensation, success misstates what happened: the effects were undone, and a parent,
  a subscription and an audit reader would read a success that did not occur. A new run keeps both facts (the failure
  and the recovery) on the record, reuses 018's audited reset and rerun with their permission checks and gate
  admission, and needs no new status or transition. (b) and (c) would add a transition from a stopped run back to a
  terminal status other than the pending one, which no reset rule, replay or `verify_run` check covers. The handler
  run's authority is its publisher's (D1177), so a handler that resets needs `run:operate` and `run:start` under that
  authority, audited like an operator's. *(Partly superseded 2026-11-25 by D022-228: no run holds its publisher's operator rights; a handler recovers through a machine identity.)*
- **D022-202 — An unmocked step of a draft test run uses the actor's current `secret:use` and access lists, audited per
  resolution, outside `prod` only (O1162 option (a)).** Delegate (OD-2026-11-24-4). Options: (a) the namespace's secrets
  under the actor's current `secret:use` and access lists, checked at each resolution and audited per resolution; (b)
  no secret at all, so every secret-declaring step must be mocked; (c) a separate set of test secrets per namespace.
  Chosen (a). The private definition's publisher is the actor (D1185), so the ordinary resolution already applies the
  actor's own rights, the 017 precedent; a revoked right takes effect at the next resolution; `prod` is excluded by
  decision D022-200, so a test run never reaches production secrets. (b) makes a test of a step that calls a provider
  impossible without a mock of the very call the author wants to test; (c) is a second secret store to classify,
  rotate and audit for a rule (a) already enforces. Each resolution writes an audit row `test_run.secret_resolved`
  (secret name and version, never a value) in the lease's transaction.
- **D022-203 — A cacheable step waits outside `READY` until its lookup is decided (question 1 of the coordinator).**
  Delegate (OD-2026-11-24-4). Options: (a) the reducer maps the first `step.released` of a node that declares `cache` to
  `WAITING` (wait kind `CACHE_LOOKUP`), and only `step.cache_hit` (with its `step.completed`) or a new
  `step.cache_missed {outcome}` (`hit` is never its outcome: `miss`, `expired`, `invalidated`, `unauthorized`,
  `bypassed`) leaves it, asked once per step of a run, failing closed; (b) the step becomes `READY` and the lookup
  happens in the same commit inside `persist`, as D1164 first said; (c) the step becomes `READY` and the claim looks the
  cache up. Chosen (a). It is the shape the slice proved for gates (ADR-0089 decision 1, ADR-0094): safety does not
  depend on a driver remembering to look up, because the reducer never makes the step `READY` before the decision, and
  the decision is a journal fact, so replay reaches the state without reading the cache. (b) cannot continue the run
  after a hit (the successors need the planner), and it is where `persist` writes the task-ready hint: today
  `crates/infra-postgres/src/execution.rs` (the `StepReleased` arm near lines 5692 to 5703) announces any step whose
  node declares no gate on its `step.released`, so a step a hit completes in the same commit would be announced to
  workers that can never lease it. (c) puts cache rows on the claim's hot path (NFR-022-007) and spends a lease to
  complete a step that needs none. The hint rule becomes "announce a step that is `READY` at the end of its commit",
  which also covers the gated case. "Asked once": a retry of a cached step, the scheduler's or an operator's, is an
  attempt of the miss and is not looked up again (the edge case "retries of a cached step are attempts of a miss").
  "Fail closed": a commit that leaves a cacheable step waiting for its lookup without a decision is refused by
  `persist` (except while its run is paused: it then waits undecided and is asked on resume, as ADR-0096 decision 2
  does for gates, because a hit is progress and a paused run makes none); a store that cannot answer fails the
  transaction, which its driver retries; it is never read as a miss. `step.cache_missed` is new vocabulary with the
  reset rule `COPY` (a payload-free fact with no row). *(Partly superseded 2026-11-25 by D022-238: the paused exception is removed.)*
- **D022-204 — The lookup round runs before the gates round; a hit takes no permit and no slot (question 2).** Delegate
  (OD-2026-11-24-4). Options: (a) the planner asks `Evaluation::CacheLookup` before `Evaluation::Gates`; a hit
  completes the step before it asks for its gates; invariant 11 and FR-022-051 admit "or a `step.cache_hit` or
  `step.mocked`"; (b) the gates first, then the lookup; (c) one combined round. Chosen (a). A concurrency limit and a
  rate limit protect what a step calls; a hit calls nothing, so holding a permit or consuming a slot for it would
  delay unrelated work and, for a rate key, waste a window's capacity on a call that is never made. (b) would make a
  hit wait behind a full key for a result it already has. (c) mixes two lock families in one round and needs one
  adapter to know both. The lock order stays the slice's: the lookup round locks cache scope and entry rows `FOR SHARE`
  after the path's entity locks and before the gates round's semaphores and windows (ADR-0056 decision 2, amended). A
  cacheable successor released after the gates round in the same commit (by a key failure's route) is asked in a later
  lookup round of that commit; that is safe because the only exclusive takers of cache rows (invalidation, purge,
  erasure and the store writer's insert) lock no run, semaphore, window or counter row after one, only usage rows last,
  so they cannot be in a cycle with a deciding transaction (the argument ADR-0050 decision 7 makes for the lease row).
  A rule of `scripts/check-architecture.py` keeps it true (ADR-0135).
- **D022-205 — A cache entry references the content-addressed `payloads` row; it holds no inline copy (question 3).**
  Delegate (OD-2026-11-24-4). Options: (a) `step_cache_entries.payload_digest` with a foreign key `(tenant_id,
  payload_digest)` to `payloads (tenant_id, digest)`; (b) an inline `output jsonb` copy, as the data model first said;
  (c) a reference to the producing run's step row. Chosen (a). `payloads` is already content-addressed per tenant and
  append-only (migration `20261009000100_execution`), and `persist` inserts outputs `ON CONFLICT DO NOTHING`: a hit's
  `step.completed` references the same digest, so no byte is copied, no second content store needs classification,
  charging or erasure, and the payload foreign key lets 020's catalog-driven discovery find every entry. The consumer
  run is still charged the output's logical bytes like any completion (`charge_logical_output_bytes`), so
  `RUN_DATA_LIMIT` and the storage-quota path apply to a hit as D1164 says. (b) duplicates up to 256 KiB per entry and
  needs its own byte triggers; (c) ties an entry to a run row that retention or archival may move. Erasure (020) must
  delete the entries of a source run before it can touch the payload; the 022-side port does that (D022-217).
- **D022-206 — The new behaviours are activated through 018's instance-capability registry (question 4).** Delegate
  (OD-2026-11-24-4). Options: (a) registry capabilities `step-cache-v1`, `early-return-v1`, `failure-handler-v1`,
  `step-executions-v1` and `draft-test-runs-v1`, required of `CONTROL_PLANE` and `EXECUTION_ENGINE`, as the slice did
  for `flow-control-concurrency-v1` and `flow-control-rate-v1` (ADR-0050 decision 9, ADR-0089 decision 6); (b) the
  per-process switches alone; (c) one capability for the whole rest of the feature. Chosen (a). Each behaviour changes
  what an engine must do at a commit (a lookup round, a handler phase, an early result, a new run origin), so an older
  binary in a mixed fleet would run a half of it: a cacheable step `READY` without a lookup, a run finished without its
  handler. The registry refuses activation while a live process lacks the capability and closes a process that lacks
  an activated one. (b) cannot see the fleet; (c) forces a rollout of everything at once and a rollback of everything
  for one defect. Until activation, publication and starts that use a capability are refused `VALIDATION_FAILED`
  `FLOW_CONTROL_ACTIVATION_PENDING` (the slice's reason, reused); the per-process switches stay as the rollback path
  (FR-022-031).
- **D022-207 — The collection- and loop-finish paths are routed through the reducer (question 5; closes G1197 when
  built).** Delegate (OD-2026-11-24-4). Options: (a) route the finish paths of `crates/infra-postgres/src/primitives.rs`
  (the collection finish writes `collection.finished`, `run.finished` and the `runs` update itself, near lines 3482 to
  3510) and `loop_progress.rs` (the loop finish writes `end.reached` and `run.finished` itself, near lines 764 to 785)
  through the planner and `persist`, so the successor's release and the run's end are decided by the same rounds as
  every other commit; (b) teach each finish path the new rules one by one; (c) keep refusing at run time. Chosen (a),
  with an interim refusal at publication only. A path that ends a run without the reducer skips everything the rest of
  022 adds at a run's end: the handler phase, the namespace dispatch, the release of a start's permit, the
  `result_already_returned` mark, and the lookup and gate rounds of a successor. (b) duplicates the terminal logic in
  three places, the "guard each caller" shape ADR-0089 rejected. (c) is today's `RuntimeNotIntegrated` refusal of a
  gated successor (retried for ever) and D022-049's refusal of a start gate with a collection: a run that fails at
  run time for a rule publication could have checked. Until the routing slice lands, publication refuses a definition
  that declares `cache`, `earlyReturn` or `onFailure` together with a `forEach`, `while` or `repeatUntil` node
  (`UNSUPPORTED_NODE`, dependency `COLLECTION_FINISH_ROUTING`); the routing slice removes the refusal and the two
  run-time refusals with its tests.
- **D022-208 — The namespace handler policy row is read plainly on the terminal path, with its `config_revision`
  recorded (question 6).** Delegate (OD-2026-11-24-4). Options: (a) a plain read (no lock) of
  `namespace_failure_handlers` in the terminal commit of a run that could qualify, the dispatch row recording the
  `config_revision` it read; (b) `FOR SHARE`, as the approved plan and ADR-0056 decision 2 say; (c) read the
  configuration at dispatch time instead of at the failure. Chosen (a). The terminal commit is the busiest commit of
  the engine; a share lock there adds a lock for every failing run of the namespace and makes every configuration
  write wait for them. What the lock bought is a stable reading, which the snapshot of the statement already gives and
  the recorded revision makes auditable: a configuration change that commits concurrently applies to that failure or
  not, by commit order, and the dispatch says which. (c) would dispatch failures that happened before a handler was
  configured, and lose the rule "clearing does not cancel obligations already committed" (spec, edge cases). The
  ADR-0056 order loses the policy row (amended 2026-11-24). *(Refined 2026-11-25 by D022-237: the revision is read with the run's definition.)*
- **D022-209 — A standalone step accepts 018's business-id policy, and its default gate group comes from the activity
  (question 7; closes G1166).** Delegate (OD-2026-11-24-4). Options for the identity: (a) the start's `businessId`,
  `businessIdConflictPolicy` and `businessIdReusePolicy` of FR-018-018..020, claimed at slot 3 of the admission order,
  besides the required idempotency key; (b) the idempotency key alone, as the approved text says "until 018 adds
  business-ID policies"; (c) a separate standalone identifier space, as Temporal's standalone activities have. Chosen
  (a): 018 is on the tree, and a one-off job is exactly the case "at most one open execution of this business key";
  (c) would be a second identifier registry. Options for the default group of a node-level gate: (a) derived from the
  executable identity (the activity name, or the script artifact's digest), so every standalone execution of that
  activity shares one key space; (b) derived from the private definition, as a workflow's default is. Chosen (a): a
  private definition is retained per actor and digest, so (b) would give each actor its own group and a declared
  `limit: 1` would limit nothing across actors. The spelling is fixed by the standalone foundation slice (task
  T022-216) under three constraints: it never equals a group a published workflow can derive (a test), it fits the
  130-character bound, and the listings' `group` filter accepts it (the pattern widened additively if needed).
- **D022-210 — A handler region is a region of the root graph, not a named body (question 8).** Delegate
  (OD-2026-11-24-4). Options: (a) `onFailure.node` names an entry in the root `nodes`, disjoint from the graph
  reachable from the entrypoint, as FR-022-012 and the IR fragment say; (b) a named body (`bodies`, the 014 shape),
  entered like a collection body. Chosen (a). A root region is scheduled by the root planner and decided by the same
  lookup and gate rounds (its nodes are root nodes for every root-only rule, so a handler step may declare `cache`,
  `concurrency` or `rateLimit`), with no per-instance scope. (b) would run the handler under 014's exact-path
  transactions, where cache and gates are refused in 0.3.0 (G1160, G1175), and would need per-instance evidence for a
  region that has exactly one instance.
- **D022-211 — Every new run creator admits through `Engine::start` (the one admission port of D022-049).** Delegate
  (OD-2026-11-24-4). The approved texts place standalone and draft-test admission "in the existing admission
  transaction" (017's `RunStore::create`) and FR-022-059 places the gate "inside `PgRunStore::create`". D022-049
  superseded that placement for the slice: the start key is an expression only the application evaluates, and
  `scripts/check-architecture.py` (`run_start_errors`) refuses a domain start call outside `Engine::start`. Options: (a)
  the four new creators (standalone, draft test, namespace handler dispatch, the derived `onFailure~h` child) call
  `Engine::start` like every creator on the tree; (b) a second port for private definitions. Chosen (a): one port is
  what ADR-0056 decision 3 requires and what the architecture rule enforces; (b) is the bypass the rule exists to
  refuse. A run-creator conformance case per new creator is task T022-221.
- **D022-212 — The gate pool is derived from the run's `test_tree` flag and threaded through every decision.** Delegate
  (OD-2026-11-24-4). About thirty sites pass the ordinary pool as a literal (`GatePool::Ordinary` or `'ORDINARY'` in
  `crates/infra-postgres/src/execution/readiness_gate.rs`, `gate_queue.rs`, `maintenance.rs`, `event_codec.rs`,
  `crates/application/src/flow_control_keys.rs`, and the two migrations' defaults). Options: (a) one function derives the
  pool from `runs.test_tree` (`GatePool::of_run`), every decision, grant, timeout, listing and journal event takes it
  from there, and a rule of `scripts/check-architecture.py` refuses a pool literal anywhere else; (b) a per-call flag
  at each site; (c) a separate set of tables for test trees. Chosen (a): one derivation cannot disagree with itself,
  and the fitness rule keeps the next site from hard-coding the pool again. (b) is thirty chances to be wrong; (c)
  doubles the gate's rows, triggers and listings for a column the tables already have. Invariant 7 of `data-model.md`
  checks the result. Task T022-204.
- **D022-213 — Every table the rest of 022 adds references runs by `(tenant_id, namespace_id, run_id)` and payloads by
  `(tenant_id, digest)`.** Delegate (OD-2026-11-24-4). The data model's cache, handler and pin tables predate the
  namespaced-foreign-key rule of the slice (plan, "Security additions": every foreign key to `runs` from a flow-control
  table is `(tenant_id, namespace_id, run_id)` to `runs (tenant_id, namespace_id, id)`, constraint
  `runs_tenant_namespace_id_uniq`). Options: (a) apply the rule to every new table (`step_cache_hit_provenance`,
  `step_cache_store_obligations`, `failure_handler_dispatches`, `test_run_pins`, `runs.handler_for_run_id`), and
  give the entries' and pins' source-run columns the namespace too, without a foreign key where archival must not cascade;
  (b) keep `(tenant_id, run_id)` for the new tables. Chosen (a): a foreign key that omits the namespace lets a row of
  one namespace name a run of another in the same tenant, which the tenant battery cannot see. A rule of
  `scripts/check-architecture.py` refuses a 022 migration whose foreign key to `runs` omits the namespace (task
  T022-205).
- **D022-214 — Where the new creators take their locks in ADR-0050's merged admission order.** Delegate
  (OD-2026-11-24-4). Plan row T16 put the draft workflow row (`FOR SHARE`, the revision check) before the 017 order,
  that is before slot 1, which no admission may do. Decided, by kind of row: the handler dispatch row is a
  run-creating aggregate (slot 3, as a pending-reset row); the failed source run's identity guard (`KEY SHARE`, ADR-0039)
  and the `fromRun` sources of a draft test (`KEY SHARE`, ascending id) are existing runs acted on (slot 3R); the draft
  workflow row is a catalog head read `FOR SHARE` (slot 4b); the pin payload rows, `test_run_pins` rows and the
  upstream `step.mocked` events are new run rows (slot 6, with the run insert); a standalone step's business-id row is
  slot 3. ADR-0050 decision 7 gains a dated amendment; plan rows T9, T15 and T16 are restated in the as-built table.
  Rejected: a recorded deviation for T16 (a lock before slot 1 is a cycle with every admission that holds a receipt
  and reads the workflow's head). *(Partly superseded 2026-11-25 by D022-222: the draft row and the handler head are read without a lock.)*
- **D022-215 — An early result is evaluated by a round, set once, and never changed after (ADR-0136).** Delegate
  (OD-2026-11-24-4). Options: (a) the planner asks `Evaluation::EarlyResult` for an `earlyReturn` node it reaches;
  `persist` writes the payload, `runs.result_digest`, `result_source`, `result_available_at`, `result.returned` and the
  `run.result_returned` outbox (none for a test tree) in that commit; a later `end` records `result_already_returned`;
  (b) evaluate the result inside `persist`. Chosen (a), the shape of `Evaluation::Output` for `end.outputExpr`, for the
  reasons ADR-0094 gives: an expression error routes (`EXPRESSION_ERROR` fails the run) and the planner continues with
  `next` in the same commit. The result is set before the run is terminal and nothing after it (a failure, a handler
  phase, a cancellation, a timeout) changes it.
- **D022-216 — The handler phase replaces `run.finished` until the handler ends (ADR-0136).** Delegate
  (OD-2026-11-24-4). Options: (a) where the planner would emit `run.finished {P}` for a run that declares `onFailure`
  with `P` in `on` and that held its start's grants (or declares none), it emits `failure_handler.started {P}`, enters
  the region (decided by the ordinary rounds) or the derived child `onFailure~h` (through `Evaluation::Child` and
  `Engine::start`), keeps the start's permit, and emits `failure_handler.finished`, the start's `concurrency.released`
  and `run.finished {P}` in one commit when the handler ends or its timer (`timeoutMs` from the start of the phase)
  fires; (b) a separate handler state machine beside the run. Chosen (a): the run stays one state machine, replay and
  `verify_run` need only the three events, and the phase inherits pause, cancellation and the rounds for free. A run
  that ends before it was ever granted its start's gates finishes directly (D1197). *(Superseded where they differ, 2026-11-25, by D022-224: the phase is its own reducer state.)*
- **D022-217 — Erasure goes through a minimal 022-side port until feature 020 calls it (task T022-063).** Delegate
  (OD-2026-11-24-4). Feature 020 is not on the tree, and neither is its registry `crates/infra-postgres/run-content-references.json`.
  Options: (a) 022 defines an application port (erase a source run's cache entries and pending store obligations; list
  the runs that received a copy by hit or `fromRun` pin) with its PostgreSQL adapter (transaction T18), and creates the
  registry file in the shape 020's data model describes ("Reference discovery") with its own entries, guarded by a
  contract test that every 022 column referencing run content is a foreign key or an entry of the file; (b) wait for
  020. Chosen (a): the copies exist from the first hit, so the means to remove them must exist with them. SC-022-016
  stays pending until 020's erasure calls the port (gap G022-021).
- **D022-218 — `TEST_RUN_UNMOCKED_EFFECT` is built with no members; FR-022-036 stays pending with no stub.** Delegate
  (OD-2026-11-24-4). Features 021, 024, 025 and 030 (the effect node kinds) and 031 (`EncodingCheck`) are not on the
  tree. Options for the effect rule: (a) the refusal mechanism with an exhaustive `match` over the node kinds that
  classifies each kind as "may run unmocked in a test" or "effect, must be pinned", with no kind in the second class
  today and a test that fails to compile when a kind is added without a class; (b) wait for those features. Chosen
  (a): each later feature then meets the rule by construction (gap G022-023). For 031 nothing is built: a stub check
  would pass by default and claim a guarantee nobody proved (AGENTS.md rule 11); FR-022-036 stays pending (gap
  G022-022).
- **D022-219 — `RESET_SOURCE_TEST_RUN` is built by the isolation slice of US06.** Delegate (OD-2026-11-24-4). The
  reason is in 018's contract and its contract test lists it as feature 022's
  (`tests/contract/schedules-bulk-operations/errors.rs`), but no code raises it. It is raised by the reset, rerun and
  batch admissions for a source with `runs.test_tree = true` in the slice that adds the flag's other rules (task
  T022-045), so the refusal and the flag land together.
- **D022-220 — Stale counts and names are corrected by dated amendments, not by rewriting approved text.** Delegate
  (OD-2026-11-24-4). The permission catalog holds 53 entries (`crates/domain/src/identity.rs`, `ALL: [Self; 53]`), so
  the three new permissions make 56, not the 51 of D1190 and T022-011; the SDKs are six (Rust since D022-016), not
  five (FR-022-025, FR-022-035, SC-022-007, SC-022-012, `contracts/conformance.md`); the scheduler's passes are in
  `crates/scheduler/src/passes.rs` (`gate_grants`, `gate_timeouts`), and `crates/scheduler/src/flow_control.rs`,
  `crates/application/src/concurrency.rs` and `rate_limits.rs` do not exist (the application side is
  `crates/application/src/flow_control.rs`); migrations are allocated after the latest on the tree (`20270209000100`
  at `d8d3cf79`; the throughput branch adds `20270210000000` and `20270210000100` for decision D022-078), not "after
  `20261220000000`"; `contracts/README.md` still says a permit-less step stays `READY`.
- **D022-221 — The build order of the rest of 022.** Delegate (OD-2026-11-24-4). Options: (a) documents; foundations
  (permissions, capabilities, quotas, fixtures, the pool, the foreign-key rule); the finish-path routing; US04 in two
  slices; US01 in four; US03 in four; US05 in two; US06 in four; then the surfaces; (b) the stories in the approved
  order (US01 first); (c) the API surfaces (US05, US06) first, since they touch the least engine code. Chosen (a). The
  routing comes first because three stories depend on a run's end going through the reducer (D022-207). US04 comes
  next because it is the smallest change to the terminal path (a result set once) and proves the round shape that
  US01 and US03 reuse. US01 before US03 because the lookup round changes readiness, which a handler region then
  relies on. US05 and US06 last among the stories because they add creators and the test-tree flag that every earlier
  rule must respect, and are tested against all of them. (b) would build the cache before the routing it needs; (c)
  would build test-tree isolation before the rules it isolates. The slices are in `tasks.md`, section "Release 0.3.0,
  the rest of 022".

### Decisions of the review of the design of the rest of 022 (2026-11-25)

The three repository reviewers read commit `14b9e40b`; their findings and the coordinating session's rulings are in
[reviews/2026-11-25/design-reviews-and-rulings.md](reviews/2026-11-25/design-reviews-and-rulings.md) and where each one
now lives is in [reviews/2026-11-25/design-responses.md](reviews/2026-11-25/design-responses.md). Each decision below
records one ruling or a group of related rulings, delegate (OD-2026-11-24-4), with the options and the reason the
rulings give. Where a ruling could not be built as written, the decision says so and states the design that keeps its
intent; nothing else changes a ruling. Decisions D022-200 to D022-221 stand except where a decision below says it
supersedes part of one. The code facts were checked on this branch at `14b9e40b`.

- **D022-222 — The draft row and the handler workflow's head are read without a lock (architecture Blocker 1;
  supersedes the slot-4b `FOR SHARE` of D022-214).** Delegate (OD-2026-11-24-4). Publication locks `workflows FOR
  UPDATE` and then the namespace's secrets (`crates/infra-postgres/src/definitions.rs:618-642`, `quotas.rs:306-312`,
  `database/admission.rs:209`), and version activation locks the row first too (`versioning.rs:534-537`); a test
  admission of a draft with a managed database node takes the secrets at slot 4a and then the row: a cycle. Options:
  (a) read the draft row without a lock, compare its `draft_revision` with the request's (a moved draft is `409
  CONFLICT`, `DRAFT_REVISION_CHANGED`) and copy what was read into an immutable private definition; (b) take the row
  before slot 1, which no admission may do; (c) a recorded deviation with the row at 4b. Chosen (a): the private
  definition is a copy of what the statement read, so a draft saved a moment later cannot change what the test runs,
  and nothing waits on the row. The handler workflow's head in the dispatch (T9) is a plain read for the same reason.
  Evidence: a case in `tests/fault/schedules-bulk-operations/slot_order.rs`, the publication of a workflow with a
  database node against a test admission of its draft (task T022-224).
- **D022-223 — The terminal commit inserts the dispatch row as a recorded deviation, always (architecture Blocker 2,
  Major 8, minors 5 and 7; correctness minor 8).** Delegate (OD-2026-11-24-4). The dispatch row is a run-creating
  aggregate (slot 3), which ADR-0050 decisions 3 and 7 forbid a terminal transition to write. Options: (a) a recorded
  deviation in ADR-0050 decision 7 with its no-cycle argument, and a fitness rule; (b) a second transaction after the
  terminal commit, which loses the "exactly one obligation per failure" of one commit; (c) the dispatch derived later
  by a pass scanning failed runs, which adds a scan and a race with the configuration. Chosen (a). The row is new: no
  transaction can wait for it before the commit makes it visible, its key is the failed run, whose lock serializes
  every writer of it, and the dispatcher locks only committed rows `SKIP LOCKED`. A rule of
  `scripts/check-architecture.py` refuses an insert into `failure_handler_dispatches` outside the terminal path of
  `persist` and a lock of it outside the dispatcher and the redrive (task T022-225). The terminal commit always inserts
  its row: the pending-dispatch ceiling refuses a new or re-enabled handler configuration and raises an alert, never a
  terminal commit. The dispatcher locks the row inside `RunStore::create` through a `NewRun` field, as `pending_reset`
  and `resets::lock_pending` do. Each attempt is counted (attempts and `next_attempt_at`) in its own committed
  transaction before the admission is tried, so a crash inside the admission still reaches `DEAD` after 12 attempts. A
  future run deletion (feature 020) locks a run and then its dispatch rows, the reverse of the dispatcher: recorded
  under gap G022-021 for 020 to order.
- **D022-224 — The handler phase is its own reducer state (correctness Blocker 1, architecture Major 4 and minors 8, 9
  and 13; supersedes D022-216 where they differ).** Delegate (OD-2026-11-24-4). As built, a stopped run drains
  (`crates/domain/src/execution/plan.rs:306-308`, `:523`, `:602-606`), a second stop is refused (`state.rs:4636-4639`,
  `409` at `crates/application/src/runs.rs:3344`), a stopped run reads `CANCELLING` (`state.rs:433-434`), and
  `check_finish` refuses `run.finished` while a step is active or with a status other than `SUCCEEDED` without a stop
  (`state.rs:5133-5163`): no handler work can run under the stop. Options: (a) a phase of its own: `failure_handler.started`
  moves the stop into `pending_terminal_status` and clears the stop, so the run reads `RUNNING` or `WAITING` and the
  planner plans the region; (b) run the handler as a separate run always (the child form only), which drops the region
  form the specification approved; (c) relax the drain for handler nodes, which puts an exception into every drain rule.
  Chosen (a). In the phase: a region step that fails without `onError` ends the phase `FAILED` and never opens a second
  phase; a cancellation or the run deadline during the phase is a handler cancellation, and the handler's own timer a
  handler timeout: a draining sub-state (`failure_handler_state` `CANCELLING` or `TIMING_OUT`) asks the region's leased
  attempts and the `onFailure~h` child to stop, and its last settle commits `failure_handler.finished`, the start
  permit's release and `run.finished` with the pending status; the handler timer exists only during the phase and
  `failure_handler.finished` cancels it; a stop request during the phase is accepted as that cancellation, not refused.
  One spelling of the timeout: `FAILURE_HANDLER_TIMEOUT` in the journal, `TIMED_OUT` in the column (the mapping is in
  `data-model.md`). Publication refuses `compensateWith` in a region (no new compensable work after the stop,
  constitution III) and a child form whose workflow's start permit shares the parent's group and a key that can
  coincide (it could never be granted while the parent holds its permit; `FAILURE_HANDLER_SELF_GATE`). Evidence:
  `crates/domain/tests/failure_handler.rs`, one test per case the reviewer lists (a gated region step, a region step
  failing without `onError`, a cancellation, the run deadline, the timeout with a leased attempt), task T022-227.
- **D022-225 — A hit requires the consumer's publisher to be able to use every secret that produced the entry
  (security Blocker 1, correctness minor 1, architecture minor 3, security minor 4).** Delegate (OD-2026-11-24-4).
  The fingerprint and the entry carried no producer secret, and the recheck read the consumer's declared secrets only,
  so a hit could hand out a result another publisher's secret produced. Options: (a) the fingerprint includes the
  producing node's sorted logical secret references, the entry records them, and a hit requires the consumer's
  publisher to be able to use every one now (metadata only), else `unauthorized`; (b) entries never shared between
  publishers, which removes most `workflow` and `namespace` scope hits. Chosen (a). The recheck takes the existing lock
  functions `sagat_lock_secret_access` and `sagat_lock_memberships` (migration `20261126000000_authority_read_locks`),
  shared, in the lookup round before the cache rows, as feature 017 fences an admission: a plain read would leave a
  revocation window the platform does not accept elsewhere. Evidence: two publishers, one access list (task T022-236).
- **D022-226 — Shared scopes cannot be poisoned (security Major 1, security minor 6).** Delegate (OD-2026-11-24-4).
  Ruled: the canonical input-mapping expression and the worker binding enter the fingerprint, and runs of internal
  definitions (standalone steps, test trees) use only the `version` scope of their own private version. The second
  half is built as ruled; a test-tree run whose version is not its own private one (a child that runs a published
  version) bypasses the cache (`bypassed`, nothing read). The first half cannot be built as written: on this tree no
  `activity` or `script` node declares an input mapping (its lease carries the whole run data document, research
  observation 1; `workflow.schema.json`) and the worker binding is namespace configuration
  (`crates/domain/src/binding.rs`), not a node field. The design that keeps the ruling's intent: (a) the fingerprint
  covers the node's whole canonical IR except a closed list of fields that cannot change what runs (`next`, `onError`,
  `cache.ttlSeconds`, `cache.name`, `concurrency`, `rateLimit`, `retry`, `timeoutMs`, `compensationRetry`,
  `compensationTimeoutMs`), so an input mapping, a binding or any field a later feature adds enters it by construction
  (fail closed: a new field splits entries, never shares them); and (b) the entry records the producing worker
  identity from the completed attempt's lease, and a hit requires that identity to hold now a binding that covers the
  activity (a metadata check), else `unauthorized`, so an entry a revoked or compromised worker produced stops being
  served. Rejected: the binding's configuration revision in the fingerprint (every binding edit in the namespace would
  invalidate every entry, and it says nothing about who produced the value).
- **D022-227 — Unreviewed code runs only where the namespace allows it (security Major 2).** Delegate
  (OD-2026-11-24-4). Draft test runs skip publication approvals, and standalone steps exempt every activity and script
  from the four-eyes check. Options: (a) test runs refused in `prod`, and in a namespace whose effective
  `publication_policies.required_approvals > 0` unless its publication policy enables draft executions (a new setting,
  off by default, changed through the publication-policy route with its permission and audited); standalone activity
  and script steps follow the same rule; feature 017's database executions keep their accepted behaviour; (b) refuse
  every test run where approvals are required, which removes the review use of draft tests the product sells; (c)
  allow them silently, which runs unreviewed code with real secrets. Chosen (a). Refusals: `403 FORBIDDEN`,
  `TEST_RUN_REVIEW_REQUIRED` and `STANDALONE_STEP_REVIEW_REQUIRED` (task T022-229).
- **D022-228 — Handler runs are a tree of their own, dispatched at most once per lineage, with no built-in authority
  (security Major 3, correctness Majors 6 and 7, correctness minors 6 and 7; supersedes D022-201's "under its own
  authority").** Delegate (OD-2026-11-24-4). Recovery and handler descendants could loop without bound, and "under its
  publisher's authority" is ambient authority. Decided: a run carries `handler_tree`, set for a dispatched handler run,
  an `onFailure~h` child, every child of a handler-tree run, and every reset or rerun whose source is a handler-tree run
  or whose initiator is one; a handler-tree run is never dispatched to the namespace handler. A namespace handler is
  dispatched at most once per lineage (the source chain of resets and reruns, `runs.lineage_run_id`), enforced by a
  unique index on the dispatch rows. `handler_kind` (`DISPATCHED`, `REGION_CHILD`) says which kind of handler run a run
  is; `handler_for_run_id` and its unique index apply to dispatched runs only; a rerun or reset of a handler run
  inherits neither. There is no built-in reset authority in 0.3.0: a handler that recovers calls the public API with a
  machine identity it is given as a secret, audited as that identity; no row of the authorization table grants a run
  its publisher's operator rights. "Qualifies" for the namespace handler means: no workflow handler phase of the run
  ended `SUCCEEDED`, and the run was not cancelled (a run cancelled during its handler phase is not dispatched: the
  operator decided). Options weighed by the rulings: the lineage index against a depth counter (a counter in the run
  needs every creator to copy it; the index is one constraint) and the machine identity against a built-in grant
  (ambient authority a reviewer cannot see). One part is not observable on 0.3.0: a request a handler makes with a
  machine identity does not carry the handler run's identity, so "whose initiator is a handler-tree run" is known only
  for creators inside the engine; the lineage index is what bounds the loop of a handler that reruns its failed run.
  Its cost, that the failure of a rerun a person starts is not dispatched either, is asked as O1176. *(Amended
  2026-11-25 by D022-241, O1176 answered (b): the lineage links only through machine identities and handler-tree runs.)*
- **D022-229 — Cache keys follow the key classification rules (security Major 4, security minors 2 and 3).** Delegate
  (OD-2026-11-24-4). FR-022-057 extends to `cache.key`: a key over a `secret` location is refused
  (`CACHE_KEY_SECRET`), over `pii` warned (`CACHE_KEY_PII`); its label, the entry counts and a hit's source run are
  shown only to a caller who may disclose every location the key reads (on the cache listing, the run's journal and the
  hit provenance; others see `null`). Keyed digests gain `KeyPurpose::StepCache` with its own bound: 1 KiB by default,
  4 KiB ceiling (`crates/application/src/keyed_digest.rs` has `Concurrency` and `RateLimit` today). Rejected: one
  purpose for all keys (a cache key and a gate key of the same value would share a digest across purposes).
- **D022-230 — The store is correct against invalidation, erasure and dead entries (correctness Majors 1 to 4,
  architecture Major 3, correctness minor 5, security minors 1 and 9).** Delegate (OD-2026-11-24-4). A result computed
  before an invalidation could be stored after it and served live; erasure and the store could lose a delete or
  deadlock; a dead entry blocked a re-store; the obligation's key digest had no source. Decided: an entry's `created_at`
  is the producing completion's instant (its store obligation's `created_at`) and it expires from it; T4 inserts the
  scope row when absent and sets `invalidated_at = clock_timestamp()` read after its lock; T5 also deletes the pending
  store obligations of the same scope and key digests; T3 skips (counted) an obligation created at or before the scope's
  `invalidated_at`, and deletes a conflicting dead entry under its lock before it inserts. T18 locks the run's store
  obligations first, then the entries by source run, and the rule of ADR-0135 decision 7 reads "after a cache row, only
  usage rows" (checked by task T022-211). The lookup's key digest and key version are kept with the miss on the step
  projection (`steps.cache_lookup`, written with `step.cache_missed`), T2 copies them into the obligation, and only the
  outcomes `miss`, `expired` and `invalidated` write one. A full pending-obligations ceiling at T2 is skipped and
  counted, never a failed completion; expired entries count against the entries ceiling until purged; the counts are
  usage rows kept by `SECURITY DEFINER` triggers with `SET search_path = pg_catalog, pg_temp`, as
  `flow_control_counts`. The runtime role may `UPDATE (expires_at)` of an entry only, and the `ALWAYS` trigger that
  refuses updates stays (it refuses any other column, and a raised expiry); never a blanket grant. Payload foreign keys
  are `RESTRICT`; erasure of shared bytes is a feature 020 obligation (G022-021). The store writer takes no lease and no
  fence: `SKIP LOCKED` is its only exclusion. Evidence: barrier tests of T3 against T4 and T5, of T18 against T3, a
  dead entry replaced (task T022-231).
- **D022-231 — A hit that would exceed the run's retained output fails its step, through the outputs path
  (correctness Major 5, architecture minor 2).** Delegate (OD-2026-11-24-4). A hit beyond the run's retained-output
  limit would fail the commit and be retried for ever. Decided: the lookup round checks the payload's size against the
  run's retained bytes and fails the step `RUN_DATA_LIMIT` through its route, as `evaluate_run_result` does
  (`crates/application/src/execution.rs:1846`); the hit's bytes enter through the outputs path of `persist`, so the
  charge, the digest and the projection come from one place (the content read is a budget line).
- **D022-232 — Every end of a run goes through the reducer, enforced, in its own ADR (correctness Major 8,
  architecture Major 5 and minor 15).** Delegate (OD-2026-11-24-4). The first list of finish paths outside the reducer
  was incomplete. Every site is now listed in task T022-207: `crates/infra-postgres/src/primitives.rs` near 3495 (the
  collection finish's `run.finished`), 4026 to 4045 (a direct `task.ready` outbox and `run.finished`) and 4061;
  `loop_progress.rs` near 778 and 3409 to 3449; `chain_progress.rs` 2246 to 2248 (the chain's `run.finished` and
  `chain.finished` outbox, allow-listed because `continueAsNew` is refused with every 022 policy). A rule of
  `scripts/check-architecture.py` refuses a `run.finished` journal or outbox write, a `finished_at =` or a direct
  `task.ready` outside `persist`, with that one allow-listed exception (task T022-226); T022-207 adds a loop and a
  collection that end `FAILED` with `onFailure` and with a namespace handler. Decision D022-207 is recorded in
  [ADR-0146](../../docs/adr/ADR-0146-every-end-of-a-run-goes-through-the-reducer.md) (proposed), cross-referencing
  ADR-0030 and feature 014's plan.
- **D022-233 — Pins route without activating, and a test run's disclosures keep the pins' floor (correctness Major 9,
  security minor 5, security nits).** Delegate (OD-2026-11-24-4). Upstream pins would have activated their
  successors. Decided: an upstream pin commits `step.mocked` and `step.routed` with no activation; `startAt` is recorded
  in `run.started` (`start_at`), so replay reaches the same entry. The highest floor among a test run's pins applies to
  every disclosure of that test run, not only to the pinned outputs. The check order of a test-run request is fixed:
  the namespace from the authenticated context, `404` for the workflow, then the `prod` or review `403`, then a
  `fromRun` `404` before `PIN_NOT_DISCLOSABLE`. A standalone script node accepts only an artifact of a published script
  version. Evidence: a domain test with `A → X` and `A → B → C`, `startAt = C` (task T022-233).
- **D022-234 — The commit order of ADR-0056 decision 2 is restated as built (architecture Major 1).** Delegate
  (OD-2026-11-24-4). The sentence of ADR-0056 decision 2 that a semaphore transaction "MUST have written that run's
  `runs` row first" is replaced, not contradicted: the transaction holds the run's row lock; a commit that ends the run
  locks its active-run counter row before it gives back any key; the `runs` row is written by `persist` after the
  rounds. A domain property test checks that no progress emits a filed gate decision and `run.finished` in one commit
  (task T022-234).
- **D022-235 — Cache rows have their slot at admission (architecture Major 2, correctness nit 3, architecture nit on
  slot 6).** Delegate (OD-2026-11-24-4). ADR-0050 decision 7 slot 7 reads: run-scope semaphores and windows, then cache
  scope, then entry (`FOR SHARE`), then step-scope semaphores and windows. Slot 6 says that `persist` writes the journal
  rows after slot 7 (the slot names the lock order; those inserts lock nothing another transaction waits for). ADR-0135
  decision 7 states the exclusive-taker argument instead of an order a run start's grant does not keep.
- **D022-236 — A capability is served only once all of it is built (architecture Major 6).** Delegate
  (OD-2026-11-24-4). Options: (a) a binary lists a registry capability only once the last behaviour slice of that
  capability has merged, with a test per capability that it is not listed before; (b) activate each slice as it lands,
  which serves half a behaviour. Chosen (a) (task T022-235).
- **D022-237 — The hot path does not move for a plain run (architecture Major 7, minors 10 and 14, security nit on the
  guard; refines D022-208).** Delegate (OD-2026-11-24-4). The namespace handler's revision and enabled flag are read with
  the run's definition (the statement that loads it), so a failing run in a namespace without a handler adds no
  statement and a qualifying run adds only its dispatch insert; the revision recorded is the one read there. The cache
  hit's content read is a budget line. `runs.test_tree` is immutable by a `BEFORE UPDATE OF test_tree` trigger, `ENABLE
  ALWAYS` (never a row trigger on every update), and the database enforces at insert that a child's flag equals its
  parent's. The evidence of a seeded `DRAFT_TEST` waiter moves from task T022-204 to task T022-217.
- **D022-238 — A cacheable step is never released while its run is paused, so the paused exception goes (architecture
  minor 1, correctness minors 2 and 3; supersedes the paused part of D022-203).** Delegate (OD-2026-11-24-4). The ruling
  removes ADR-0135 decision 6 if no path first-releases a root step while the run is paused. Checked: while paused the
  planner returns `Plan::Idle` after deciding the gates of steps the retry pass or an operator made ready
  (`crates/domain/src/execution/plan.rs:309-316`, the idle return at `:316`), and forward planning is off (`:101`), so no `step.released` is
  emitted while paused; the exception is removed. `step.cache_missed` joins the task-ready hint's arms. Invariant 14
  admits `step.mocked`.
- **D022-239 — Migrations and rollback (architecture minors 11 and 12).** Delegate (OD-2026-11-24-4). The `origin`
  CHECK of `runs` is extended `NOT VALID` and validated by a second migration; the unique index on `runs (tenant_id,
  handler_for_run_id)` is built in its own migration with its lock measured on the acceptance data set (a `CONCURRENTLY`
  build outside a transaction if it exceeds the budget); every `down` refuses while rows with the new origins, flags or
  events exist. Once a journal holds the new events an older binary cannot decode the run
  (`crates/infra-postgres/src/event_codec.rs:490`): rollback is switching the capability off, never a binary downgrade
  (task T022-237).
- **D022-240 — Contracts, bounds and wording (security minors 7, 8 and 10, correctness minor 4, correctness nit 4,
  architecture minors 4, 6 and 16, architecture and security nits).** Delegate (OD-2026-11-24-4). A handler workflow
  that is missing, foreign or internal (a private definition) is the same opaque `404 NOT_FOUND`; only a pinned version
  that is not published stays `422 INVALID_FAILURE_HANDLER`. `keyValue` of an invalidation is bounded at 4 KiB and the
  JSON depth limit; `mock` keeps the pin bound of 256 KiB with the JSON depth limit (the ruling's "4 KiB" read for
  `keyValue`; a 4 KiB mock would contradict the approved pin bound of the ceilings table, so the reading is stated in
  the response file). SC-022-002 and US01 scenario 4 are restated against the cache-entries ceiling (an entry charges no
  content). ADR-0135 names the interim `COLLECTION_FINISH_ROUTING` refusal for `cache`. T15 and T16 place the retained
  definition where `create` takes it, `ad_hoc::retain` with its workflows quota lock before the slot-4a locks
  (`crates/infra-postgres/src/execution.rs:1531` against `1619-1640`), a deviation inherited from 017 that ADR-0050's
  amendment records and task T022-224's slot-order case proves. ADR-0135 decision 2 reads "the transaction's data,
  including outputs staged in the same commit"; decision 9 says the writer takes no lease or fence; "obligation" is
  qualified everywhere (018's, store, dispatch). The shared standalone default group is documented; the cache listing's
  exposure of private workflow ids to `ops:read` is stated, and its `sourceRunId` follows D022-229 (null for a caller
  who may not disclose the key's locations), which reconciles the security nit with security minor 3. The draft-test
  pool's own load on a shared external system is a section of the runbook (task T022-061). `FOR SHARE` on a hot entry
  writes a multixact per hit: measured by task T022-062. This branch merges after `docs/owner-delegation-2026-11-24`,
  which records OD-2026-11-24-4.
- **D022-241 — A lineage links only through machine identities and handler-tree runs (O1176 option (b); amends
  D022-228).** *(Reversed 2026-11-25 by D022-242: a machine identity is not a principal kind of the platform; O1176 is
  answered (a), with suppressed dispatches written, shown and redrivable.)* Delegate (OD-2026-11-24-4), the coordinating session's answer to O1176. Options: (a) at most one dispatch
  per lineage whoever started the rerun, as first ruled; (b) a reset or rerun extends its source's lineage only when its initiator is a machine identity or a handler-tree run; a reset or rerun a person (a member) starts begins a lineage of its own; (c) at most N dispatches per lineage.
  Chosen (b). A person's rerun of a failed run is a deliberate human decision and its failure may reach the namespace
  handler again, which is what an operator expects; recovery by a handler goes through a machine identity (D022-228), so
  the loop the reviewers found stays bounded; (a) would silently suppress alerts for a person's reruns, and (c) is an
  arbitrary number that still suppresses some. `runs.lineage_run_id` is therefore the source's `lineage_run_id` when the
  reset's or rerun's initiator is a machine identity or a handler-tree run, and the run's own id otherwise (a member, or a
  run that is no reset or rerun); the initiator is the authenticated principal of the request, whose kind (member or
  machine identity) the admission knows, and it is recorded with the reset or rerun provenance. The unique index on the
  dispatch rows' lineage is unchanged. Gap G022-028 is narrowed accordingly.


### Decisions of the re-review of the design of the rest of 022 (2026-11-25)

The three reviewers re-read `6d02632e`; their findings and the coordinating session's second rulings are in
[reviews/2026-11-25/design-rereviews-and-rulings.md](reviews/2026-11-25/design-rereviews-and-rulings.md), and where each
finding lives is in [reviews/2026-11-25/design-reresponses.md](reviews/2026-11-25/design-reresponses.md). Each decision
below records a ruling or a group of related rulings, delegate (OD-2026-11-24-4). Where a ruling cannot be built as
written, the decision says so and states the design that keeps its intent. Decisions D022-200 to D022-241 stand except
where a decision below says it reverses or supersedes part of one.

- **D022-242 — Every reset and rerun extends its source's lineage; a suppressed dispatch is written, shown and
  redrivable (rulings "Lineage" 1 and 2; the section's findings are Architecture N0 and N11, Correctness B1 and M5, Security A,
  B and C; reverses D022-241, so O1176 is answered (a)).** Delegate (OD-2026-11-24-4). "A machine identity" is not a principal kind of the platform
  (`crates/domain/src/identity.rs:152`; the only machine roles cannot start runs, `:688-690`), so D022-241 cannot be
  built; and the lineage unique index made the always-insert terminal commit fail with a unique violation and retry for
  ever. Options: (a) every reset and rerun extends the lineage, whoever starts it, and a dispatch the lineage suppresses
  is a row of its own, counted, listed and redrivable; (b) D022-241, which needs a principal kind that does not exist;
  (c) no lineage bound, which lets a handler loop. Chosen (a): an operator sees and controls every suppressed handler,
  nothing loops, and nothing depends on a principal kind the platform lacks. The two rulings, read together, are not
  buildable literally: an `ON CONFLICT DO NOTHING` (ruling 2) writes no row, so nothing would be left to show and
  redrive (ruling 1); and a redriven suppressed row would conflict with a unique index "over the live rows". The design
  that keeps both: the dispatch row has `lineage_first boolean`; the unique index is `(tenant_id, namespace_id,
  lineage_run_id) WHERE lineage_first`; the terminal commit inserts its row with `lineage_first = true ... ON CONFLICT
  (tenant_id, namespace_id, lineage_run_id) WHERE lineage_first DO NOTHING`, and only when that inserted nothing, a
  second statement inserts the row with `lineage_first = false` and state `SUPPRESSED_LINEAGE` (counted
  `skipped_lineage`; the outcome journaled on the run); a redrive turns a suppressed row `PENDING` and keeps
  `lineage_first = false`, so it never conflicts. Both statements are the last of the run's own commit before the usage
  rows; ADR-0050's recorded deviation extends to the lineage key: the first statement may wait for another run's
  uncommitted first row of the lineage, and that wait is a leaf wait, since after the insert the commit takes only rows
  of its own run and usage rows. Evidence: two failing runs of one lineage commit concurrently, both reach their
  terminal status, exactly one `lineage_first` row and one suppressed row (task T022-238).
- **D022-243 — A namespace dispatch budget bounds a handler that starts fresh runs (ruling "Lineage" 3; same section).** Delegate (OD-2026-11-24-4). A handler that starts a fresh run, a trigger or a Signal-with-Start escapes
  the lineage. Decided: a namespace dispatch budget (dispatches a namespace may admit per window, with a per-workflow
  share; quota resources) defers a dispatch beyond it, never drops it, and raises an alert; runs a dispatched handler
  starts through its own `child` nodes are handler-tree runs, and so are the runs of `child` nodes of a handler region
  (ruling "Handler phase" 5). Rejected: refusing fresh starts from handler runs (a notifier that opens a ticket workflow
  is the main use).
- **D022-244 — A built-in role `RunRecovery` is the only credential ADR-0136 accepts for recovery (ruling "Lineage" 4;
  same section; supersedes D022-228's "a machine identity").** Delegate (OD-2026-11-24-4). The prescribed credential, an
  Operator or a Developer subject, can approve publications and decide approvals, defeating four-eyes review
  (`identity.rs:650-675`, `versioning.rs:465`). Decided: `RunRecovery` holds `run:read`, `run:start` and `run:operate`
  and no publication, approval, version or secret permission; a subject whose only role is `RunRecovery` is refused as a
  publication approver and as an approval decider (a check in both deciders, beyond the missing permissions). Task
  T022-011's catalog counts the role. No run is ever granted its publisher's rights.
- **D022-245 — `lineage_run_id`, `handler_tree` and `handler_kind` are filled and guarded by the database (ruling
  "Lineage" 5; Security minor 5).** Delegate (OD-2026-11-24-4). `lineage_run_id` is nullable, read as
  `coalesce(lineage_run_id, id)`, filled by a `BEFORE INSERT` trigger from the reset or rerun source (so an older binary
  and the chain insert keep working), and immutable; `handler_tree` and `handler_kind` get the guards `test_tree` has (a
  `BEFORE UPDATE OF` trigger, `ENABLE ALWAYS`; a child's `handler_tree` checked against its parent's at insert).
- **D022-246 — Draining is a journaled event, and every stop source in the phase maps to it (rulings "Handler phase" 1
  and 2; the section's findings are Correctness B2, M1, M2, M3 and minors 2, 5, 6, 7, 8, and Architecture N3).** Delegate (OD-2026-11-24-4). A draining sub-state without a
  journal fact cannot be replayed. Decided: one event `failure_handler.draining {cause}`; during the phase every stop
  source (`run.stop_requested`, the storage-quota stop, a region step failing without `onError`, an expression error in
  a region condition or switch) maps to it and never to the run's stop; the first cause wins and later ones are recorded
  in its detail; `runs.failure_handler_state` follows the journal (a domain case rebuilds the state from the journal
  after each commit); draining overrides a pause as a stop does; the recovery sweep also selects draining runs. The
  ruling's causes are `CANCELLED`, `RUN_DEADLINE`, `TIMEOUT`, `REGION_FAILED`, `STORAGE_QUOTA`, `RUN_DATA_LIMIT`,
  `EXPRESSION_ERROR`. One of them cannot occur: ruling "Handler phase" 3 takes the run deadline out of the phase, so
  `RUN_DEADLINE` is unreachable. The design that keeps both rulings coherent: the closed set is the other six; a
  `RUN_DEADLINE` cause is not in the codec, and a decoder test pins the six. Rejected: keeping an unreachable value
  (a reader would believe a deadline can drain a handler).
- **D022-247 — The run deadline does not apply during the phase (ruling "Handler phase" 3; Architecture N3;
  supersedes the deadline half of D022-224).** Delegate (OD-2026-11-24-4). Otherwise the next overdue
  pass cancels a `TIMED_OUT` handler. Decided: only the handler's own timer bounds the phase (D1174), `timeoutMs` has its
  ceiling (3,600,000 ms), and the overdue pass (`crates/infra-postgres/src/maintenance.rs`, `overdue_runs`) excludes
  `failure_handler_state IS NOT NULL`; a deadline that fires before the phase opens is the run's stop, and the phase then
  opens normally.
- **D022-248 — Qualification, region children, the handler timer and the cases (rulings "Handler phase" 4 to 6;
  same section).** Delegate (OD-2026-11-24-4). A run qualifies for the namespace
  handler when no workflow handler phase ended `SUCCEEDED` and no operator asked to cancel it; a draining cause other
  than `CANCELLED` qualifies. The derived `onFailure~h` node cancels and awaits its child. T7 creates the handler timer;
  T8 cancels it and holds the region step's gates in its sorted set. The cases: a parallel region with one branch
  failing while the other is leased; a storage-quota stop in the phase; paused, then the timeout; `on: [TIMED_OUT]`; a
  cancellation during `TIMEOUT` draining and a timeout during `CANCELLED` draining, both orders (task T022-227).
- **D022-249 — Continue-as-new is refused with every new policy and for every isolated tree (ruling "Finish paths" 1;
  the section's findings are Architecture N4, N6, N7 and Correctness M4 and minor 3).** Delegate (OD-2026-11-24-4). A successor would escape `test_tree`, `handler_tree`
  and the cache. Decided: publication refuses `continueAsNew` in a definition that declares `cache`, `onFailure` or
  `earlyReturn` (`FLOW_CONTROL_COMBINATION` extended), and `refuse_gated_successor`
  (`crates/infra-postgres/src/chain_progress.rs`) refuses a successor of a `test_tree` or `handler_tree` run.
- **D022-250 — A chain's finalization reaches the namespace dispatch (ruling "Finish paths" 2; same section).**
  Delegate (OD-2026-11-24-4). The chain finish skipped the dispatch. Decided: the chain finalization's terminal write
  goes through `persist`'s terminal path for the namespace dispatch, so a failing chain head is dispatched like any run.
  ADR-0146's allow-list keeps the chain hand-off and finalization for what they write otherwise, with its reason
  rewritten: the namespace handler is not a declaration, so "no 022 policy is declared with `continueAsNew`" was not a
  reason for the dispatch.
- **D022-251 — The finish-path fitness rule is defined by enclosing function, and ADR-0146 has a mixed-version and a
  held-lock test (rulings "Finish paths" 3 and 4; same section).** Delegate (OD-2026-11-24-4).
  The first rule could not pass (`persist`'s helpers write the very statements it refuses). Decided: the rule judges the
  enclosing function (`enclosing_function`, as `run_finished_errors` does), allow-lists the `persist` family (`persist`,
  `persist_addressed`, `persist_scoped_addressed`, `outbox_task_ready*`, `outbox_run_finished`), the body iteration
  announcements (with their reason) and the chain hand-off and finalization, restricts the `finished_at` clause to
  `UPDATE runs`, and has negative fixtures. ADR-0146 adds a test in which an older binary replays a journal the new path
  finished, and a held-lock test against concurrent feature 014 iteration transactions.
- **D022-252 — The cache store, completed (rulings "The cache store" 1, 2, 4, 5, 6, 9, 11 and 13; the section's findings are
  Correctness M6, M7, M9, M10, minors 1 and 4 and nits, Architecture N10, Security minors 1 to 4 and 7 to 9; supersedes
  the stamp of D022-230).** Delegate (OD-2026-11-24-4).
  Decided: the entry is stamped with the database instant of the producing attempt's lease (`step.leased`) and expires
  from it, so a value computed by an attempt leased before an invalidation is never stored as live; an invalidation by
  key writes a per-key tombstone (`step_cache_key_invalidations`: scope digest, key digest, `invalidated_at`) that T3 and
  the hit read; hits staged in one round count cumulatively, in round order, against the retained-output limit; one
  usage row holds both counts (entries and store obligations), so T3 and T18 cannot deadlock on them; invariant 18 is
  restated as "no `step.cache_hit` names an entry that was dead at the hit's commit", which a test can fail; store
  obligations keep one column `UPDATE` grant guarded by an `ALWAYS` trigger, so `FOR UPDATE` works as `sagat_app`;
  an invalidation inserts a scope row only for a scope a published version can derive, and any other target is `404`;
  `UPDATE (expires_at)` is one of the exclusive takers of ADR-0135 decision 7, and two writers replacing one dead entry
  use `ON CONFLICT DO NOTHING`. Evidence: a barrier test with T2 held open across T5; two hits in one round; a barrier
  held after T3's delete against T18 (task T022-231).
- **D022-253 — `step.cache_missed` is re-materialized by a reset (ruling "The cache store" 3; same section;
  supersedes the `COPY` rule of D022-203).** Delegate (OD-2026-11-24-4). A reset copies `steps.cache_lookup` from the source step, so a store
  obligation of the reset run's next completion has its key digest.
- **D022-254 — The cache's security, completed (rulings "The cache store" 7, 8, 10 and 12; same section, and
  Architecture N5 for the slot).** Delegate (OD-2026-11-24-4). Decided: entries record secret ids, not names; the binding check
  stores the producing worker's subject, takes the binding advisory lock shared (`pg_advisory_xact_lock_shared`, as
  `crates/infra-postgres/src/bindings.rs:21`) after the secret locks and before the cache rows, checks the worker's
  membership, and, for a script, checks the producing runner's subject and its binding profile; for a caller who may not
  disclose every location the key reads, `entry_id`, the expiry and `entriesDeleted` are null and listing instants are
  cut to the minute (as D022-060); spans and metrics carry no key, label or source run (their attribute sets are listed
  in the plan); namespace-scope cache names are declared resources of the namespace (an administrator's setting that
  lists the workflows that may write the name; any workflow of the namespace may read), which closes poisoning through
  fields a key does not read, while `version` and `workflow` scopes need no declaration; the lookup's secret-access,
  membership and binding locks are taken at slot 5 for an admission (the definition is known there), and ADR-0056
  decision 2(a) lists them with the leaf argument for a later round. Evidence: a `slot_order.rs` case, a cacheable
  admission against a revocation (task T022-224).
- **D022-255 — The order is restated as built everywhere, and the handler configuration is read with the run's row
  (rulings "Orders and statements" 1 and 2; Architecture N2, N5, N9; supersedes D022-237's "read with the run's
  definition").** Delegate (OD-2026-11-24-4). The order of a transition on an existing run is: entity locks, then the
  rounds (lookup, then gates), then `persist` (the active-run counter row when the commit ends the run, the give-backs,
  the rest), then usage. ADR-0056 decision 2's clauses on the policy row `FOR SHARE` and "`runs` update, then semaphore"
  are struck with a dated line; the no-cycle argument for the counter row after cache rows is written (deciders lock
  cache rows shared only, and no exclusive taker of a cache row takes the counter row); ADR-0094 is cited as the
  authority. There is no statement that "loads the run's definition" to join: the handler configuration (statuses,
  revision, enabled flag) is read by a left join in the statement that locks the run, with `FOR UPDATE OF runs` so the
  configuration row is not locked; the definition cache carries nothing mutable; the budget line is corrected.
- **D022-256 — A capability is served by a manifest of required artefacts (ruling "Capability serving"; Architecture
  N8, minor 1; supersedes the build constant of D022-236).** Delegate (OD-2026-11-24-4). A constant set by a slice proves
  nothing (a tautology). Decided: a per-capability manifest names the artefacts that must exist (event types the codec
  decodes, `Evaluation` variants, migrations, routes, the named acceptance test present and not ignored), and a test
  fails when the serving constant is true while any is missing. Rollback order: `platform_requirement_remove`, then the
  switch; a switch gates new use only, never phases or obligations in flight.
- **D022-257 — Draft executions are read under the policy's lock, and `prod` refuses standalone code steps too (ruling
  "Draft executions"; Security minor 6).** Delegate (OD-2026-11-24-4). `draft_executions_enabled` is read inside the
  admission transaction under the publication policy's existing lock; runs already admitted continue when it is turned
  off (documented); creating the policy row to set the flag keeps the environment's approval default; a rerun of a
  standalone run is checked again. The ruling says "`prod` refuses standalone activity and script steps as it refuses
  test runs, unless the setting is enabled". Read as: in `prod`, standalone activity and script steps are refused unless
  the setting is enabled (`STANDALONE_STEP_PRODUCTION_NAMESPACE`), and draft test runs stay refused whatever the
  setting, as O1160 (D022-200) decided; the "unless" does not reopen test runs in `prod`.
- **D022-258 — The rest of the minors and nits (ruling "The rest").** Delegate (OD-2026-11-24-4). The interim
  `COLLECTION_FINISH_ROUTING` refusal is kept as a defence after the routing and said so; reruns by unrelated automation
  extend the lineage, a consequence stated with the suppressed and redrivable row; a budget line for the child
  `test_tree` check; a Complexity Tracking row for the recorded deviation, and the Constitution Check re-dated; the path
  is `crates/infra-postgres/src/definitions.rs` (the citations corrected); the plan's persist fact states the counter row
  before the give-backs; the old text of T022-215 is marked superseded; D022-238 cites `plan.rs:316`;
  `FAILURE_HANDLER_SELF_GATE` refuses the direct case only, and longer cycles are bounded by the queue and handler
  timeouts, stated; the plan's authorization paragraph is reworded; an `unauthorized` outcome is shown as `bypassed` to a
  run reader who may not see the producer; a namespace-scope key's classification is the producer's, stated; `SECURITY
  DEFINER` triggers schema-qualify `public.` tables; T17's fence is `sagat_lock_namespace`, never the `namespaces` row.
- **D022-259 — One governing transaction table (Architecture N1, Correctness M8, Security D).** Delegate
  (OD-2026-11-24-4). `plan.md`'s table "Transactions, as built" (section "Release 0.3.0, the rest of 022") is the one
  that wins; its rows T1, T3, T5, T8, T9, T15, T16 and T18 are rewritten whole with every ruling, not by appended
  remarks; the original table of "Transaction boundaries" is marked superseded at its heading, each of its rows having a
  row of the same number in the governing table. Every document that still carried an old row is corrected.


### Decisions of the third reads (2026-11-25)

The reviewers read `c910345b` for Blockers and Majors only; none found a Blocker, and every verdict on every ADR was
"accept" or "accept with changes". The third rulings are in
[reviews/2026-11-25/design-third-reads-and-rulings.md](reviews/2026-11-25/design-third-reads-and-rulings.md) and where each
lives is in [reviews/2026-11-25/design-third-responses.md](reviews/2026-11-25/design-third-responses.md). Each decision is
delegate (OD-2026-11-24-4).

- **D022-260 — `run.finished` is appended after the dispatch statements (ruling 1; architecture Major 1, correctness
  Major 1).** Delegate (OD-2026-11-24-4). The `run.finished` journal row is appended after the two dispatch statements,
  its `namespace_dispatch` detail taken from what the inserts did; it is a row of the run's own commit, so the leaf
  argument of D022-242 holds; the reducer ignores the field. The terminal path updates no `run_events` row. Evidence:
  the held-commit case of task T022-238 asserts each run's `run.finished.namespace_dispatch` equals its dispatch row's
  state.
- **D022-261 — A further stop during draining is refused, and there is one cause (ruling 2, option (a); correctness
  Major 2; supersedes the "later causes" of D022-246).** Delegate (OD-2026-11-24-4). Options: (a) while a run drains, a
  further stop request (an operator's cancellation, the deadline, a second timeout) is refused `409`, as a second stop
  is today, region step failures settle into the drain and add no cause, `later_causes` is removed, and qualification
  reads the one journaled cause; (b) a journaled event per later cause. Chosen (a): (b) makes qualification depend on an
  unbounded sequence for no operator benefit. Evidence: cancellation and timeout in both orders, the dispatch row and
  the state rebuilt from the journal agreeing.
- **D022-262 — The overdue pass is a typed no-op on a run in its handler phase (ruling 3; correctness Major 3).**
  Delegate (OD-2026-11-24-4). Between the pass's pick and its lock a commit may open the phase; `stop_run(TimedOut)` on
  such a run is a typed no-op the pass counts as skipped. Evidence: a barrier test with the pass held between pick and
  apply while the commit that opens the phase lands; no `failure_handler.draining` follows.
- **D022-263 — Continue-as-new under a handler or test tree is refused at admission (ruling 4; correctness Major 4;
  refines D022-249).** Delegate (OD-2026-11-24-4). Refused with typed codes where the run would be created: a dispatch
  fails with `last_error_code` `CONTINUE_AS_NEW_REFUSED`; a child admission under a handler or test tree fails its child
  step with `CONTINUE_AS_NEW_REFUSED`; a draft test of such a definition is refused `422 VALIDATION_FAILED`
  (`CONTINUE_AS_NEW_REFUSED`). The runtime check of `refuse_gated_successor` stays as a defence and ends the run through
  a typed finalization, as the chain-limit finalization of `crates/scheduler/src/passes.rs` does. Evidence: no run stays
  non-terminal at the boundary.
- **D022-264 — A deferral by the dispatch budget is not an attempt (ruling 5).** Delegate (OD-2026-11-24-4). A deferral
  moves `next_attempt_at` and counts nothing; only an admission tried counts. Evidence: a dispatch deferred more than
  12 times stays `PENDING` and is admitted once.
- **D022-265 — A hit's producer locks: an unlocked pre-read, then the locks, then a re-check (ruling 6; architecture
  Major 2; refines D022-254 and rewrites ADR-0056 decision 2(a)).** Delegate (OD-2026-11-24-4). Options: (a) an unlocked
  pre-read of the entry's immutable `producer_subject`, then that subject's membership and binding locks shared, then
  the entry `FOR SHARE` and the subject checked again (a change is `unauthorized`); at an admission the pre-read and
  locks at slot 5 and the re-check at slot 7; (b) a try-lock. Chosen (a): (b) would turn contention into spurious misses.
  ADR-0056 decision 2(a) is rewritten on the true premise: a binding replacement locks `worker_sessions` and `steps`
  after its key (`crates/infra-postgres/src/bindings.rs`), so the binding lock is not a leaf; the rule is that no decider
  locks a step or a `worker_sessions` row before its lookup round, stated and checked. Evidence: a `slot_order.rs` case
  of a binding replacement against a lease-expiry pass that releases a cacheable successor.
- **D022-266 — `RunRecovery` is bound to namespaces, reads definitions, never redrives, and waits for activation
  (ruling 7; security Majors 1 and 2, architecture Major 3; refines D022-244).** Delegate (OD-2026-11-24-4). A
  `RunRecovery` subject is bound to named namespaces (an administrator's audited setting, as worker bindings are); every
  route that authorizes it checks the binding first and answers the opaque `404` outside it; the role also holds
  `definition:read` (the test `every_role_that_may_start_a_run_may_read_the_definition_it_starts`,
  `crates/domain/src/identity.rs:1091`, stays as it is); it may not redrive a dispatch (`403`,
  `RECOVERY_ROLE_MAY_NOT_REDRIVE`); a migration widens `memberships_role_valid` with a `down` that refuses while such
  memberships exist; the role cannot be assigned until `failure-handler-v1` is activated; the OpenAPI role enum is
  widened in the same change. Evidence: a tenant-battery case (bound to namespace A, cannot touch namespace B); a fault
  test (a handler that reruns and then tries to redrive produces exactly one handler run).
- **D022-267 — The fitness rules land in the slices that make them pass (ruling 8; architecture Major 4).** Delegate
  (OD-2026-11-24-4). The finish-path rule (task T022-226) lands in slice 3, its pull request's first commit showing it
  red; the dispatch-insert rule (T022-225) in slice 10; the evidence of T022-245 and of T022-207's handler cases in
  slices 11 and 12; slice 3 keeps a test that a chain finalization goes through `persist`'s terminal path.
- **D022-268 — ADR-0146 decision 1 states the order as T31 (ruling 9; architecture Major 5, the open part of N2).**
  Delegate (OD-2026-11-24-4). Chain, run, the scope's rows, the rounds, `persist` (the counter row when the commit ends
  the run, the give-backs, the rest), usage.
- **D022-269 — The draft-executions setting has an advisory lock (ruling 10; refines D022-257).** Delegate
  (OD-2026-11-24-4). An advisory key per namespace, held shared at slot 4a by admissions that read the setting and
  exclusively by `set_policy` (`crates/infra-postgres/src/versioning.rs`), which locks nothing after it but its row and
  audit.
- **D022-270 — Leftovers (ruling 11; Security A, Correctness M8, Architecture N0).** Delegate (OD-2026-11-24-4). The
  plan's machine-identity paragraph and the two failure-matrix rows of D022-228 and D022-241, the data model's
  `lineage_run_id` row and dispatch note of D022-241, and ADR-0136's old text (the run-deadline case under "Enforced by",
  "the revision read with the definition" in option 8, the consequence it carried) are struck or marked superseded by
  D022-242; the mapping of Security B and C in `design-reresponses.md` is corrected (B to D022-244, C to D022-242).
- **D022-271 — ADR-0135, ADR-0136, ADR-0137 and ADR-0146, and the feature 022 amendments of ADR-0050, ADR-0056 and
  ADR-0094, are accepted.** Delegate (OD-2026-11-24-4). After three review rounds (`reviews/2026-11-25/`: the reviews of
  `14b9e40b`, the re-reviews of `6d02632e`, the third reads of `c910345b`), with no Blocker in the third and every
  verdict "accept" or "accept with changes", the coordinating session accepts the four ADRs and the amendments once
  D022-260 to D022-270 are written into them, without a fourth full review. Each status line reads `Accepted
  (2026-11-25, delegate OD-2026-11-24-4, after three review rounds recorded in
  specs/022-flow-control-step-policies/reviews/2026-11-25/)`. Task T022-200 records it as an evidence line and is not
  ticked by this change.

## Gaps

| ID | Gap | Status |
|---|---|---|
| G1160 | Cache and step concurrency in bodies need per-instance evidence (014 exact-path transactions). | Open; `UNSUPPORTED_NODE` |
| G1161 | Blob outputs are not cached, mocked or pinned (ADR-0023 reference counting needed). | Open |
| G1162 | A parent `child` node cannot complete on the child's early result. | Open |
| G1163 | Rotating the flow-control digest key is refused while semaphore holders or waiters exist; no live re-keying. | Open; runbook |
| G1164 | Database query results are not cacheable (ADR-0035 decision 10). | Open |
| G1165 | Grants are FIFO by admission commit until 019; with 019 the order is D1602. | Dependency |
| G1166 | Standalone business-ID policies are 018. *Closed 2026-11-24 by design (decision D022-209): a standalone step accepts 018's `businessId` and its conflict and reuse policies; built by task T022-216.* | Closed by design; T022-216 |
| G1167 | Hot-key semaphore contention is unmeasured (NFR-022-004). | Open; benchmark |
| G1168 | FIFO follows admission commit order, not event arrival (G74 stands). | Accepted |
| G1169 | Approval nodes cannot be mocked. | Open |
| G1170 | No handler for `CANCELLED`. | Accepted (D1173) |
| G1171 | `step.cache_hit` names its source run: run readers learn two runs had equal keys. *2026-11-25 (decision D022-229): the source run is shown only to a caller who may disclose every location the key reads; others see `null`.* | Accepted (narrowed, D022-229) |
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
| G1197 | The collection- and loop-finish paths (`crates/infra-postgres/src/primitives.rs`, `loop_progress.rs`, `crates/scheduler/src/passes.rs`) write their successor's step row directly: a gated root successor of a `forEach` or a loop is refused there (`RuntimeNotIntegrated`, fail closed; the finish is retried) instead of being decided. Corrected 2026-11-19: the first text said they refused an unexpected evaluation, which was true only for an `end` successor. *2026-11-24 (decision D022-207): the two finish paths are routed through the reducer by task T022-207, which closes this gap; until then publication refuses `cache`, `earlyReturn` and `onFailure` with a `forEach`, `while` or `repeatUntil` node (task T022-206).* | Open; T022-207 |
| G1198 | No deployment configuration carries the digest keys (`KeyRing`) to the stores of the control plane and the engine: a gated step is refused `BINARY_CAPABILITY_MISSING` without them. Lands with the capability listing (D022-037). *Extended 2026-11-19 (D022-045): a scheduler replica needs the ring too; without it the grant pass grants nothing (`unsupported`) and keeps its keys due; the timeout routine does not need it.* *Closed 2026-11-20 (D022-057): the ring is the file `SAGAT_FLOW_CONTROL_DIGEST_KEYS_FILE` in both binaries (a switch on without it does not start); both list `flow-control-concurrency-v1` with the switch and the ring; the admission point admits a gate a process serves. Open: the rate gate's listing (T022-072, T022-074).* | Closed for the concurrency gate; rate with T022-074 |
| G1199 | The readiness gate applies the declared `limit` and `queueTimeout`; the namespace's `concurrency_limit` and `flow_control_queue_timeout_ms` ceilings (`FlowControlCeilings`) are not read there yet, nor the live-keys ceiling (`FLOW_CONTROL_LIMIT` of FR-022-054) or a waiters-per-key bound (security review of 2026-11-19, Major): with distinct keys a caller can create key rows without bound in one namespace. No process may list the capabilities before these are applied. *Partly closed 2026-11-19 (D022-045): the namespace limit, queue timeout and live-keys ceilings apply at readiness (`FLOW_CONTROL_LIMIT`); step waiters are bounded by the plan per run and gated node, not per key. Open: a namespace ceiling of rate-limit keys and values (T022-077); run waiters per key (T022-022, T022-072).* *2026-11-20 (D022-057): the concurrency half is met: run waiters per key are bounded at admission (`CONCURRENCY_QUEUE_FULL` under the key's lock, `run_admission::*`) and `queued_runs` is exact (D022-051), so the concurrency capability is listed; the rate half (T022-077) stays open and the rate capability unlisted.* *2026-11-20 (D022-064): the rate half is met: `rate_limit_live_keys` and the per-window limits apply at readiness and admission, and the rate capability is listed.* | Closed (D022-057, D022-064) |
| G022-001 | The readiness gate records no metric or span yet (`concurrency_acquisitions_total`, `rate_limit_decisions_total`, `workflow.concurrency.acquire`, `workflow.rate_limit.admit`, rule 10; architecture review of 2026-11-19, M3). A grant counts as unleased until the pass records its lease (T022-074), so a rate-limited step key admits only `limit` steps in all until then. A key that yields `null` shares the group's single key. *Partly closed 2026-11-19 (D022-045): decisions, grants, timeouts, dequeues and returns of the step scope are counted after commit and spanned by group. Open: the wait histograms and waiter gauges, the grant-lag and unleased-grant series and their alerts (T022-082).* | Open; T022-082, T022-074, T022-104 |
| G022-002 | The live-keys ceiling is checked without a namespace lock: concurrent transactions that each create a new key at the boundary can pass it by at most the new keys the others create (their number minus one when each creates one key). *Accepted 2026-11-19 by the owner (O1172 (a), OD-2026-11-19-4) for 0.3.0; the bound is documented in `docs/runbooks/flow-control.md` (D022-047). The exact bound is in keys when a round creates several (O1173), accepted 2026-11-20 (OD-2026-11-20-3).* *Extended 2026-11-20 by the owner to `rate_limit_live_keys` (OD-2026-11-20-11, "Yes, extend it"; D022-066 item 6), which closes the security reviews' suspicion 4.* | Accepted (OD-2026-11-19-4; in keys, OD-2026-11-20-3; rate keys, OD-2026-11-20-11) |
| G022-003 | A rate-limit grant's lease instant is recorded by a pass that is not built (transaction T23, T022-074): a leased grant counts for ever, so a rate key that reached its limit frees only by a returned slot or a dequeue, and its window never rests until an age-out instant. T022-097's rate half rests on it. *Closed 2026-11-20 (D022-064 items 4 and 5): the lease pass records the instant; the rest rule waits for the age-out of every waiter's window.* | Closed; T022-074 |
| G022-004 | `verify_run` does not check that a `wait_timed_out` comes at or after the deadline its `*.queued` journaled; the reservation counts an eligible waiter ahead without its own position (conservative). | Open |
| G022-005 | An orphaned waiter (of a run that ended) is logged and counted (`outcome="orphan"`), not alerted; the runbook does not list the `gate_grant` and `gate_timeout` action kinds. | Open; T022-061, T022-082 |
| G022-006 | The digest-key commands (activation, force, listing) are SQL functions with no audited API, CLI, span or metric (NFR-022-005, constitution VI; security review of 2026-11-20, finding 4); the force's history is append-only only for roles that may not drop its trigger, and a deployment where operators share the schema owner's login records whatever name they type and can retire a version by a direct `UPDATE` with no record (finding 5). The runbook tells operators to use their own login and never to edit the table by hand. | Open; T022-061, T022-082; a dedicated operator role |
| G022-007 | FR-022-021 says "pausing a holder MUST NOT release its permit", and FR-022-019 counts a granted, unleased step as a holder, while FR-022-046 says a pause returns the permit and slot of a granted, unleased step: the approved texts disagree for that case; the build follows FR-022-046 (D022-048). T022-025's "pause keeps permit" holds for a leased step and a run holder. `data-model.md` ("Steps and their gates", invariant 11) does not say that a step a pause gave back waits with no waiter row. Revised when the approved texts are next re-approved. | Open; re-approval of the texts |
| G022-008 | A step queued when its run pauses keeps its waiter row and its deadline, which runs during the pause: a long pause can fail it `CONCURRENCY_WAIT_TIMEOUT` or `RATE_LIMIT_WAIT_TIMEOUT` (the timeout routine does not look at the pause), while a step granted before the pause waits with no deadline (correctness review 4 and architecture M2 of 2026-11-20). Not introduced by T022-100. | Open; O1175 (a) accepted without option (c) (OD-2026-11-20-2), so the deadline rule is a later decision |
| G022-009 | A start delay or a reset of a definition that gates its start is refused `BINARY_CAPABILITY_MISSING` (D022-049): the plan says the gate is decided at the delay's release, and a reset takes a fresh permit or is refused `CONCURRENCY_PERMIT_UNAVAILABLE` (never queues); neither is built. A child node whose run's start is refused (`CONCURRENCY_QUEUE_FULL`, `FLOW_CONTROL_LIMIT`) is retried by the next pass instead of failing its node with the typed retryable code (T022-102). *Closed in part 2026-11-20 (T022-064): the delay is decided at its release (D022-052) and the reset takes a fresh permit or is refused (D022-053). Open: the child node's typed code (T022-102).* | Open; T022-102 |
| G022-010 | A Signal- or Update-with-Start of a definition whose start queues is refused `SIGNAL_NOT_ACCEPTED_AT_START` (no step waits while the entry is held), so the outcome depends on the key's load (correctness review M2 of 2026-11-20). Chosen design (D022-050 item 6): hold the signal or update with the queued run and deliver it at the grant. *2026-11-20 (T022-064):* an Update-with-Start is not affected (it changes the declared state, which a queued run has: `run_creators::an_update_with_start_admits_through_the_key_and_applies_its_update`); a Signal-with-Start keeps the typed refusal with nothing created (`run_creators::a_signal_with_start_whose_start_would_queue_is_refused_and_creates_nothing`). The hold-and-deliver needs a durable held signal, its delivery in the grant's transaction and the replay of its receipt: follow-up T022-109. | Open; T022-109 |
| G022-011 | `queued_runs` was believed counted without a lock (D022-050 item 3). *Closed 2026-11-20 (D022-051):* the count is exact. Only an admission adds a run waiter, every admission holds the namespace's active-run quota lock from before its runs row to its commit, and the start's gate counts after it in a fresh READ COMMITTED statement; proved by `run_admission::two_starts_at_the_queued_runs_ceiling_are_decided_one_after_the_other` (the lock held by the test, both starts observed waiting on it, then exactly one queued and one refused), red under two hand mutations (the lock removed; the ceiling ignored). The merge blocker the owner set on it is met. T022-105's counter rows remain for the cost of the count (a scan of the namespace's run waiters, at most the ceiling) and its privileges. | Closed (exactness); T022-105 open for the counter rows |
| G022-012 | The start's decision (`decide_start_gates`) and the step's (`decide_gates`) are two code paths over the same rules; T022-072 builds one core for both levels and both gates (D022-050 item 5; architecture review Major 1 of 2026-11-20). A paused queued run's deadline runs during its pause, as G022-008 says of a step. *2026-11-20 (D022-054): the start's three causes share one validator (`check_start_decisions`) and one adapter function; the step's decision is still its own path.* *Closed 2026-11-20 (D022-064 item 2): `gates_of` and `decide_filed` serve both levels and both gates.* | Closed; T022-072 |
| G022-013 | Keys-review correctness m5 and m8 (2026-11-20): the takeover test races a raw `UPDATE` of the shard lease, not a keeper's takeover, and checks "nothing twice" afterwards; and SC-022-004 at run level lacks five same-key starts with `limit: 1` (at most one holder, grants in admission order) and a mixed-limit pair sharing a group. | Open; T022-110 |
| G022-014 | Without the label, a key's counts and the oldest-wait order still let a caller who may not reveal personal data partly link runs (one key of a group with waiters: every `WAITING` run of the group waits on it). The labels are hidden and the instants truncated to the minute (D022-060), which narrows and does not stop linking (D022-061). An aggregated listing per (pool, scope, group) for such callers closes it: T022-111 (D022-061). *2026-11-20 (D022-067 item 8):* `getRun`'s `rateLimit.position` is another surface of the same class (how many runs of the key are ahead); the estimate is withheld from such callers. *2026-11-21 (T022-086 security re-review, nit):* one bit more of the same class, a key's fullness: the synchronous `409` of a reset refused `RATE_LIMIT_SLOT_UNAVAILABLE` or `CONCURRENCY_PERMIT_UNAVAILABLE`, and the `failureReason` of the reset register (`run_resets.failure_reason`), which shows it afterwards to anyone who can read the entry; no code change. | Open; T022-111, a release item with T022-104 |
| G022-015 | Publication does not yet refuse a concurrency key over a `secret` location (T022-104), while this build lists and can activate the concurrency capability (D022-057). The runbook forbids production activation before it; the release cannot close with it open. | Open; release gate, T022-104 |
| G022-016 | A reset of a run whose start declares `rateLimit` is refused `BINARY_CAPABILITY_MISSING` (D022-064 item 7): `RATE_LIMIT_SLOT_UNAVAILABLE` (018's code, G1181) and the reset rules of the `rate_limit.*` events land with T022-086. The refusal is retryable and no process of this build lifts it, so a pending reset of such a run is retried until it is cancelled: the runbook forbids production activation before it (D022-066 item 5), and `run_creators::a_reset_of_a_rate_limited_start_is_refused_until_its_rules_are_built` pins it. *Built 2026-11-21 (T022-086, D022-069):* a reset takes a fresh slot right after `run.reset` or is refused `RATE_LIMIT_SLOT_UNAVAILABLE`, rate first, and the five `rate_limit.*` events have the rules of `contracts/journal-events.md`; the interim refusal and its test are gone. *Closed 2026-11-21:* T022-086 ticked after its re-review (three reviewers, 0 Blockers, 0 Majors). | Closed (T022-086) |
| G022-017 | The purge keeps a grant for its window row's longest period seen (`max_period_ms`) plus a minute: an acquirer that later declares a longer window on the same key (a `RATE_LIMIT_MISMATCH` warned at publication) does not count the grants purged before it arrived, and may be admitted up to their number early in its first window. | Open; documented in the runbook |
| G022-018 | The grant purge visits keys in no order, without key-shard partitioning or an index on the lease instant (a full scan of `rate_limit_grants` a call): under steady per-second traffic on more keys than it visits per housekeeping its backlog grows (reviews of 2026-11-20, architecture Major 5). *2026-11-20 (D022-066 item 1):* the rows of a namespace are bounded by its `rate_limit_live_keys` again, and a decision that would be refused reclaims its own namespace's purgeable windows of at most 64 grants first, so only a window of more grants waits for this purge. Open: the order, the shards, the index, a time budget, an `outcome="failed"` and an oldest-purgeable-age gauge; a call stops at the first round that deleted nothing, which a round of keys all held by others also is (re-review correctness minor 3). | Open; production gate (runbook section 6); T022-062, T022-089 |
| G022-019 | A reset with the strategy `CANCEL_SOURCE` of a rate-limited source whose key's window is full cancels its source and then fails `RATE_LIMIT_SLOT_UNAVAILABLE`, with no run replacing it (D022-069 items 3 and 8; reviews of 2026-11-21, architecture Major 1, security minor; re-review architecture minor C). The runbooks warn and advise `REQUIRE_TERMINAL`; the two contracts say so. Task T022-114 builds the advisory refusal at the request, or a warning in the reset preview. | Open; not a production gate (D022-069 item 8); T022-114 |
| G022-020 | A namespace's `concurrency_limit` lowered while runs are queued for a permit: the grant routine compares a queued waiter with the limit filed at its admission, as the rate gate did before D022-071; the specification's edge case speaks of a new version's lower limit only (`spec.md`, US02 edge cases). Whether a lowered concurrency ceiling applies to queued waiters at once is a product question for the coordinator (OD-2026-11-20-6) before it is built. *Closed 2026-11-21 (decision D022-074):* the grant from the queue evaluates a permit waiter, and a waiter ahead, against the lower of its filed limit and the current `concurrency_limit`; held permits are never revoked; `lifecycle::a_lowered_concurrency_ceiling_applies_to_the_queued_starts_and_revokes_no_permit`. | Closed; D022-074 |
| G1189 | Nothing in the database computes or checks `key_shard` and the waiter `shard`; two binaries that hashed differently would file one key under two shards. | Open; T022-097 |
| G1190 | `flow_control_key_activate` records the operator as typed and keeps no history row with the database role, unlike the registry's activations. *Partly closed 2026-11-16 (D022-042): the version row records `activated_role` (`session_user`); a history of earlier activations of the same version is not kept, since a version is activated once. 2026-11-19 (D022-046): every force has an append-only history row with the database role and a reason; the runbook part is `docs/runbooks/flow-control.md`.* | Open; T022-061 (the rest of the runbook) |
| G1191 | Untested: the down migrations' refusal when `run_events` holds a `concurrency.*` or `rate_limit.*` event; no contract test parses 018's reset-rule table against `RULES`. | Open |
| G1192 | An older replica's quota views read every stored limit and refuse a resource name its build does not know (`stored_limits`): a tenant that sets one of the new limits gets an error from an older replica's quota listing until it is upgraded. True of every resource added before. | Accepted; the next resource change could make the reader skip unknown names |
| G1193 | Activating a digest key version that a live process was not configured with makes that process refuse every gated decision (`UnknownVersion`) until it is; the registry does not check the key versions processes hold. | Open; ADR-0093, runbook (T022-061) |
| G1194 | The pinned operation baseline of `contract_iceberg_writes` digests each operation without following `$ref`, so a change to a shared parameter or schema (as the quota enum of D022-040) moves no digest; `contract_flow_control_step_policies::quotas` pins this enum to the code instead. | Accepted |
| G1195 | Approved texts still describe the drain-first rotation: FR-022-020's last sentence, `data-model.md` (key versions, "only by an audited operator command"), G1163; `data-model.md` does not list `flow_control_group_declarations` or `activated_role`; `contracts/errors.md` does not say that `*_POLICY_INVALID` covers the namespace's ceiling of node-level declarations. Changing them makes the docs drafts again (constitution, Approval). *O1170 answered 2026-11-19 (OD-2026-11-19-2). Also out of date since the force (D022-046): FR-022-056's "no key is ever split between two rows" and SC-022-028 (a forced rotation splits a key into at most two rows), and `data-model.md` lacks `flow_control_key_forced_retirements`. The texts, and the bound of D022-047 in `contracts/errors.md`, are revised when the approved texts are next re-approved (a change makes them drafts again).* | Open; re-approval of the texts |
| G1196 | A rotation's progress (rows left under the retiring version) has no metric and no audit record; the operator learns it by running the command again. *2026-11-19: likewise the rows left under a forced version (an over-admission in progress, D022-046); a force has an audit row, an activation does not.* | Open; T022-061, T022-082 |
| G1183 | A database clock that moves backwards under-delivers until it catches up (NFR-022-008); no alert tells the operator why grants paused beyond `rate_limit_grant_lagging`. | Accepted; runbook |
| G022-021 | Feature 020 is not on the tree: the 022-side erasure port (decision D022-217) deletes a source run's cache entries and pending store obligations and lists the runs that received a copy, but nothing in production calls it, so SC-022-016 cannot pass (added 2026-11-24). *2026-11-25 (decisions D022-223, D022-230): also for 020 to order, a run deletion locks a run and then its dispatch rows, the reverse of the dispatcher; and payload foreign keys are `RESTRICT`, so erasing shared bytes is 020's obligation.* | Dependency (020); T022-212 |
| G022-022 | Feature 031 is not on the tree: FR-022-036 (`ENCODING_READ_BY_CACHE_KEY`, `earlyReturn.result` as a carriage position, mocks, pins, standalone inputs and cache entries as write paths) is not built and has no stub (decision D022-218; added 2026-11-24). | Dependency (031) |
| G022-023 | Features 021, 024, 025 and 030 are not on the tree: `TEST_RUN_UNMOCKED_EFFECT` has no member kind; each of those features adds its kind to the exhaustive classification, which fails to compile until it does (decision D022-218; added 2026-11-24). | Dependency (021, 024, 025, 030); T022-218 |
| G022-024 | The namespace handler policy row is read without a lock (decision D022-208): a configuration change that commits while a run's terminal commit is in flight applies to that failure or not by commit order; the dispatch row records the revision it read. A handler cleared just after a failure's commit is still dispatched for that failure, as the edge case "disabling does not cancel obligations already committed" says (added 2026-11-24). | Accepted; documented in the runbook (T022-061) |
| G022-025 | Until the finish-path routing lands (task T022-207), publication refuses `cache`, `earlyReturn` and `onFailure` in a definition with a `forEach`, `while` or `repeatUntil` node (`UNSUPPORTED_NODE`, dependency `COLLECTION_FINISH_ROUTING`; decision D022-207; added 2026-11-24). | Open; T022-206, closed by T022-207 |
| G022-026 | A cacheable step released while its run is paused waits for its lookup undecided and is looked up on resume (decision D022-203): an entry that expires during the pause is a miss on resume (added 2026-11-24). *Closed 2026-11-25 (decision D022-238): no root step is released while its run is paused, so the exception is removed.* | Closed (D022-238) |
| G022-027 | A handler step that declares `concurrency` or `rateLimit` waits for its gates during the handler phase, and the phase's own `timeoutMs` keeps running: a handler behind a busy key can time out `FAILURE_HANDLER_TIMEOUT` without running (decision D022-210; added 2026-11-24). | Accepted; documented |
| G022-028 | A request a handler makes with a machine identity does not carry the handler run's identity, so "a reset or rerun whose initiator is a handler-tree run" is known only for creators inside the engine; the lineage index bounds the loop (decision D022-228; added 2026-11-25). *Narrowed 2026-11-25 (decision D022-241): the lineage now follows the request's principal kind, which the admission knows; what stays unobservable is only whether a machine identity acts for a handler, which does not matter, since every machine-identity rerun extends the lineage.* *Closed 2026-11-25 (decision D022-242): the lineage no longer depends on who started a reset or rerun, so nothing about the initiator needs to be observed; a handler's fresh starts are bounded by the dispatch budget (D022-243).* | Closed (D022-242, D022-243) |
| G022-029 | The fingerprint's input mapping and worker binding of the ruling name fields no node declares on this tree; they are covered by the canonical node digest with a closed exclude list and by the producing worker's binding check (decision D022-226; added 2026-11-25). | Accepted by design |

## Owner questions

### O1160 — May draft test runs execute in `prod` namespaces?
**Answered 2026-11-24 by delegation (delegate, OD-2026-11-24-4; decision D022-200): option (a) — refused in `prod` (`FORBIDDEN`, `TEST_RUN_PRODUCTION_NAMESPACE`); `dev` and `staging` allow them; production debugging is 018's reset.**
Unmocked steps are real executions. **Recommended default: no** (`TEST_RUN_PRODUCTION_NAMESPACE`);
production debugging uses 018's reset. An opt-in would need its own audit design.

### O1161 — May a failure handler mark the run recovered?
**Answered 2026-11-24 by delegation (delegate, OD-2026-11-24-4; decision D022-201): option (a) — no; a handler never changes the run's outcome, and recovery is a handler that starts a reset or a rerun, an audited new run.**
Windmill's `recover: true` turns a failure into success; after compensation that misstates what happened.
**Recommended default: no**; the handler never changes the outcome.

### O1162 — Which secrets may an unmocked step of a draft test run use?
**Answered 2026-11-24 by delegation (delegate, OD-2026-11-24-4; decision D022-202): option (a) — the namespace's secrets under the actor's current `secret:use` and access lists, checked and audited per resolution, outside `prod` only.**
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
### O1176 — Should the failure of a rerun or reset that a person starts be dispatched to the namespace handler?
**Answer reversed 2026-11-25 (delegate, OD-2026-11-24-4; decision D022-242): option (a) — every reset and rerun extends its source's lineage, whoever starts it; a dispatch the lineage suppresses is written as a `SUPPRESSED_LINEAGE` row, counted, listed and redrivable by an operator. The first answer below is kept as history.**
*First answer, reversed:* **Answered 2026-11-25 by delegation (delegate, OD-2026-11-24-4; decision D022-241): option (b) — the lineage links only through resets and reruns whose initiator is a machine identity or a handler-tree run; a person's rerun begins a new lineage and its failure is dispatched again.**
Asked 2026-11-25 (decision D022-228), to the coordinating session under OD-2026-11-24-4. The ruling of security Major 3
dispatches a namespace handler at most once per lineage (the source chain of resets and reruns), which bounds the loop of
a handler that reruns its own failed run. It also means that when a person reruns a failed run and the rerun fails again,
no handler is dispatched for it. Options: (a) **as ruled**, at most once per lineage, whoever started the rerun;
(b) the lineage follows only links whose initiator is a machine identity, so a reset or rerun a member starts begins a
new lineage and its failure is dispatched, while a handler's machine identity still extends its lineage; (c) at most N
dispatches per lineage. **Recommended: (b)**, if the principal kind (member or machine identity) is recorded with the
reset and rerun provenance: it keeps the loop bound the ruling wants (a handler acts through a machine identity, D022-228)
and keeps the notification an operator expects after their own rerun. Until answered, the documents follow (a).

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

## Owner decisions of 2026-11-24

Recorded in `docs/owner-decisions.md` (section of 2026-11-24).

- **OD-2026-11-24-1 — "Inclua tudo na 0.3.0".** Release 0.3.0 contains the rest of feature 022 beyond its throughput
  slice: US01, US03, US04, US05 and US06, with the tasks that serve them (T022-011, T022-057, T022-063). Their plan is
  the section "Release 0.3.0, the rest of 022" of `plan.md` and their slices the section of the same name in `tasks.md`.
- **OD-2026-11-24-4 — "Tomem a melhor decisão".** The coordinating session and the agents it briefs answer the open
  questions, accept or reject ADRs and settle review findings in the owner's place, each act recorded as "delegate
  (OD-2026-11-24-4)" with its reasoning. O1160, O1161 and O1162 are answered under it (decisions D022-200 to D022-202),
  and decisions D022-203 to D022-221 are taken under it. ADR-0135, ADR-0136 and ADR-0137 are proposed; the coordinating
  session accepts or rejects them after the independent reviews (task T022-200).

## External semantics for rate limits (not re-read on 2026-11-13)

Stated from the author's knowledge of these products, without quotation; a reviewer should confirm them
before they are cited elsewhere. Windmill's concurrency limits count executions within a time window and
queue the excess (already listed above; D1181 took only the concurrent-holder part). Temporal limits the
rate at which a task queue dispatches activities, on the server, as a smoothed rate. Inngest separates a
lossy `rateLimit` from a queued `throttle` with `limit`, `period`, `burst` and `key`. The shape here keeps
the queued meaning and the `limit`/window/`key` vocabulary, and chooses the strict window for the reason
of D022-003.

