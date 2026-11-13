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
- **D022-017 — Timers and passes.** A queue deadline is an ordinary timer on the run or step, handled by
  the existing timer pass (as T12). Grants and lease recording are one new pass in
  `crates/scheduler/src/flow_control.rs` on the scheduler tick; no new process and no new hint is needed
  for correctness (a hint only shortens the wait).

Numbers taken on 2026-11-13 after listing `docs/adr` on `origin/develop`, every `origin/*` branch and the
sibling worktrees (first as ADR-0084, renumbered by the coordinator because designs 041 and 042 took
ADR-0084..0087 the same day): ADR-0088 (ADR-0081..0083 are on `origin/feature/018-business-ids`); T022-066..090;
FR-022-039..050; NFR-022-007..008; SC-022-017..023; G1174..1183; O1163..1168.

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
| G1181 | Feature 018 lives on another branch: its reset-rule registry and its refusal reasons must gain the `rate_limit.*` rules and `RATE_LIMIT_SLOT_UNAVAILABLE` when both are on one tree; until then T022-086's 018 cases are pending, not passed. | Dependency (018) |
| G1182 | With feature 031, a rate-limit key over an encoded location must be refused by a code 031 registers (as `ENCODING_READ_BY_CONCURRENCY_KEY`). | Dependency (031) |
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

## Owner decision of 2026-11-13: throughput control in release 0.3.0

To be recorded on `develop` as **OD-2026-11-13-11** in `docs/owner-decisions.md` (not edited by this
branch). The owner asked for "Controle de througput para fluxos que tenham limitações de execução,
buffering" in release 0.3.0 and chose "Concorrência + taxa": (1) concurrency limits by key with a durable
queue, which is US02 of this feature as specified (FR-022-017..022); and (2) a rate limit per time window
with buffering (N executions per second or minute per key, the excess waits durably instead of failing),
which this feature did not specify and which is added as US07. The other user stories (cache, early
return, failure handlers, standalone steps, test mode) are not in the 0.3.0 slice. The feature itself was
approved for implementation by OD-2026-11-11-1; O1163–O1168 were answered the same day, every
recommendation accepted (OD-2026-11-13-16, "Aceitar todas", recorded on `develop` by the coordinator). ADR-0056 and
ADR-0088 stay Proposed: the owner accepts ADRs separately (T022-066).

## External semantics for rate limits (not re-read on 2026-11-13)

Stated from the author's knowledge of these products, without quotation; a reviewer should confirm them
before they are cited elsewhere. Windmill's concurrency limits count executions within a time window and
queue the excess (already listed above; D1181 took only the concurrent-holder part). Temporal limits the
rate at which a task queue dispatches activities, on the server, as a smoothed rate. Inngest separates a
lossy `rateLimit` from a queued `throttle` with `limit`, `period`, `burst` and `key`. The shape here keeps
the queued meaning and the `limit`/window/`key` vocabulary, and chooses the strict window for the reason
of D022-003.
