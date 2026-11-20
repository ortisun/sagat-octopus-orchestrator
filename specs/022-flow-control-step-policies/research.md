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
