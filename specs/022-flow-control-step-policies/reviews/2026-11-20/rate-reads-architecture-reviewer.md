# Architecture review: task T022-078 (the rate gate's reads)

Reviewer: `architecture-reviewer` (read-only, diff and spec only), 2026-11-20, over the local commit after d08bb79b on
`feature/022-throughput-control` (diff `d08bb79b..HEAD` without the generated Studio types). Text as returned;
resolutions in `rate-reads-responses.md`.

**Architecture review: T022-078, `Run.rateLimit`, `waitingOn` and `GET /namespaces/{namespaceId}/rate-limit-keys`, feature 022**

I could only read files. I ran no test, script or `git` command. Branch and worktree state comes from reading `.git/refs`, `.git/packed-refs`, `.git/worktrees/*` and the sibling worktrees' migration directories.

## Verdict

Approve with changes. Nothing blocks the merge. The layering, the contract's growth, the migration and its `down`, and the hot-path budget all hold. Two problems need fixing before T022-078 is ticked:
- The new listing makes a cost promise that has no check, and part of it does not hold.
- A new invariant (`rate_limit_windows.oldest_wait_at`) has no check that fails when it is broken.

## Blockers

None.

## Major

**Major 1: the listing's "a page costs its own keys" promise has no check.**
- Where: plan "Visibility of the rate gate" (last bullet); D022-067 item 4; `rate_limit_windows_listing_idx` and `rate_limit_windows_group_listing_idx` in `20270207000000_flow_control_rate_key_listing.up.sql:97-102`.
- The concurrency twin has one: `tests/integration/flow-control-step-policies/rest.rs:678`, `a_page_of_keys_reads_only_its_own_keys`. It runs `EXPLAIN ANALYZE` over 5,000 keys and asserts the listing index is used, with no sort and no sequential scan, reading exactly the page's rows.
- Nothing fails if `RATE_KEY_LISTING_SQL` or `RATE_KEY_LISTING_OF_GROUP_SQL` (`crates/infra-postgres/src/flow_control.rs:937,948`) stop using their indexes. A drift in a column or collation would bring back a full scan of the namespace on every page.
- Smallest fix: run that same test over the two rate statements, with 5,000 `rate_limit_windows` rows.

**Major 2: the cost of each key's `nextSlotAt` is not bounded as the plan says.**
- Where: `crates/infra-postgres/src/flow_control.rs:920-929`.
- The filter `g.leased_at + make_interval(secs => p.period_ms / 1000.0) > now()` puts an expression on the indexed column. So it reads every grant row of the key through `rate_limit_grants_count_idx`, including leased grants that have aged out and wait for the purge. That backlog is bounded only by the purge (G022-018).
- `CROSS JOIN (SELECT DISTINCT x.period_ms FROM rate_limit_waiters x …)` reads every waiter of the key. STEP-scope waiters per key have no per-key bound (plan "Ceilings, revised": `active_runs` × gated nodes).
- The plan's cost line counts only "its counted grants … at most 10,000".
- Consequence: a page of 201 keys that includes a busy STEP key (all its grants leased, many waiters) runs into `statement_timeout = 2s` and returns 503. That is the moment an operator most needs the view.
- Smallest fix, in two parts:
  - Rewrite the filter as `g.leased_at > now() - make_interval(secs => p.period_ms / 1000.0)` and take `min(g.leased_at) + p`. That is one index probe per period.
  - Bound the set of periods, for example to the first 256 waiters in `rate_limit_waiters_order_idx` order (the most one pass grants a key). Say so in the contract and the plan.
- Cover it with Major 1's test, using a STEP key with many waiters.

**Major 3: the value of `rate_limit_windows.oldest_wait_at` has no check.**
- The CHECK `rate_limit_windows_oldest_wait_iff_waiters` only enforces "null exactly when nobody waits". The "oldest wait first" order depends on the value being `min(enqueued_at)`.
- The concurrency twin is checked after every flow-control test (`tests/support/flow_control.rs:519-530`, D022-058, architecture-principles row 42). The rate column is not, and its presence is not required by a `must(...)` line (`tests/support/flow_control.rs:409-416`).
- No test deletes several rate waiters of one key in a single statement, which is the case the trigger's comment argues about.
- T022-079 owns the rate tables' consistency in general, but this column is new in this task.
- Smallest fix:
  - add the matching row ("a window's oldest wait differs from its waiters'") to `key_consistency`, guarded by `must(20_270_207_000_000, …)`;
  - add a tamper test next to `consistency.rs:197`;
  - add a row to the principles table.

**Major 4: `estimatedGrantAt` gives another run's exact instant to a caller without the reveal grant.**
- Where: `crates/application/src/runs.rs:2366-2381`; `estimated_grant` at `crates/application/src/flow_control_keys.rs:235`.
- The estimate is the k-th counted lease plus the window, which is another run's grant instant plus `per`, to the microsecond (the test asserts `granted_us(holder) + HOUR_US` exactly).
- D022-067 item 6 and D022-060 truncate exactly such derived instants to the minute in the listing. `getRun` does not.
- `listRuns` shows the untruncated `since` of every GRANTED run with its group. So the reader can tell that two runs share a key whose label is hidden. That is G022-014's linking, through a path D022-061 does not mention.
- Smallest fix: in `RunService::get`, truncate `estimated_grant_at` to the minute when the caller may not reveal; the reveal decision is already made there. Otherwise, record the exemption in D022-067 and extend G022-014.
- The security reviewer should confirm this.

**Major 5: the task's own evidence is incomplete.**
- T022-078 (`tasks.md:391`) asks for evidence that "the estimate is absent in the cases of D022-014". There is no test for a run that also waits for a concurrency permit; only "beyond one window" is tested.
- FR-022-048 says a step waiting "for a slot or a permit" must be shown. `waitingOn: "concurrency"` (`crates/infra-postgres/src/execution.rs:2331-2333`) has no test.
- Smallest fix, two tests in `rate_rest.rs`:
  - a start declaring both `concurrency` and `rateLimit`: assert `position` is set and `estimatedGrantAt` is null;
  - a step-level concurrency waiter: assert `waitingOn == "concurrency"`.

## Minor

**Minor 1: the key-value-free fitness function does not cover the new route.**
- `KEY_VALUE_FREE_FILES` (`scripts/check-architecture.py:537`) lists `dto_flow_control.rs` and `routes/concurrency.rs` only.
- `routes/rate_limits.rs` passes today but is unguarded.
- Fix: add `"crates/api-http/src/routes/rate_limits.rs"`. It is one line.

**Minor 2: reusing `RunPermitState` for a rate slot carries concurrency meaning into it.**
- `flow_control_keys.rs:88-104`: its doc says `Waiting` "fails `CONCURRENCY_WAIT_TIMEOUT`", and `as_str()` returns `"HOLDING"`. The wire value is `GRANTED`, and the DTO works around this with its own match (`dto_flow_control.rs`, `RunRateLimitBody::from`).
- A rate grant also differs in kind: it expires by time and is never given back.
- That is independent meaning, so rule 6 does not argue against a separate type.
- Fix, either of:
  - a two-variant `RunRateSlotState { Granted, Waiting { deadline } }` with its own `as_str`;
  - make `RunPermitState` gate-neutral: neutral doc, no `as_str`.

**Minor 3: the route is a copy of the concurrency route, and its names now mislead.**
- `routes/rate_limits.rs` repeats `routes/concurrency.rs` line for line. That includes the security ordering: cursor checked before admission, the group check, the in-force check after admission.
- It also reuses `ConcurrencyKeyQuery` and `concurrency_cursor_in_force` (`operations.rs:833`), which now serve both listings.
- Fix: extract the shared opening of both handlers, parameterised by `KeyListing`, and rename the two shared items to `KeyListingQuery` and `key_cursor_in_force`.

**Minor 4: the grant routine's eligibility text now exists in five places.**
- `r.status IN ('RUNNING','WAITING') AND NOT r.paused AND deadline_at > t` appears in:
  - `execution/gate_queue.rs:163,180`
  - `maintenance.rs:217,225`
  - `RATE_PLACE_SQL` (`flow_control.rs:962`)
- D022-067 item 2 says the position follows the pass's rule, but nothing keeps the five texts equal.
- The `k` formula is also written twice: once as the SQL `OFFSET greatest(n + a - L, 0)` and once in `estimated_grant`.
- Fix: one `eligible_waiter_sql!` macro, as `purgeable_grant!` does for the purge.

**Minor 5: the estimate's statement can fail the whole `getRun`.**
- The estimate is "never a promise", but a failure of `rate_place` (`runs.rs:2372`, `?`) turns `getRun` into an error. One way it can fail is the 5 s pool timeout on a key with 10,000 waiters ahead.
- Fix: on error, leave `position` and `estimatedGrantAt` null and count or log it.
- Also document the race: a run granted between the two reads shows `WAITING` with `position: null` in `getRun`, which the contract does not describe.

**Minor 6: the hot-path cost is invisible to the work budget, and the plan does not say so.**
- `work_budget.rs:33-43` counts top-level statements and rows only. So no line moves, but only because the budget cannot see the cost:
  - every waiter insert or delete now runs a nested `PERFORM … FOR UPDATE` and a `min()` probe in the trigger;
  - every update of a window that cannot be a HOT update writes two more index entries (waiter-counter and `next_eligible_at` updates already could not be, because of `rate_limit_windows_due_idx`);
  - `RUN_SELECT` adds two primary-key probes to `get`, `graph` and `check_database_replay` (`execution.rs:625`), which is on the admission path.
- Fix: one paragraph in plan "Hot-path budget of a gated definition", with T022-089 as the measure.

**Minor 7: three pieces of new logic have no unit tests.**
- `estimated_grant` is pure and has four branches: room now, inside one window, beyond one window, missing lease. It has no unit test.
- Nothing tests that `decode(encode(RateLimit), Concurrency)` returns `None`, or the reverse. The existing test is `a_position_it_did_not_write_is_refused`, `flow_control_keys.rs:521`.
- The contract promises a 400 for another group's cursor, a retired version's cursor, and a rate cursor sent to the concurrency listing. None of these is tested.

**Minor 8: nothing checks the response bodies against the canonical schemas.**
- `RunRateLimit`, `RateLimitKey` and `RateLimitKeyPage` are `additionalProperties: false` with required lists.
- `support::contract::OpenApi::validate` exists but `rate_rest.rs` does not use it. The concurrency twin does not either.
- Fix: validate `rateLimit` and the page in `rate_rest.rs`.

## Nit

- **The two contract documents disagree in two places.** In the 022 document, `Run.rateLimit` lacks `since` in `required`, and `RateLimitKeyPage` lacks `additionalProperties: false`. The canonical document has both.
- **"eligible" means two things.** The contract's position text says "eligible waiters". ADR-0089 decision 4 defines "eligible" to include "admitted by its own limit", which D022-067 item 2 deliberately leaves out. Use another word, or cite D022-067.
- **The trigger's lock is more than it needs.** `PERFORM … FOR UPDATE` (`20270207000000…up.sql:57-61`) also runs on waiter inserts, where `LEAST` needs no fresh snapshot. It is also stronger than the `FOR NO KEY UPDATE` that the UPDATE takes, so it conflicts with other transactions' foreign-key checks. The concurrency twin does the same, and the gate already holds `FOR UPDATE`, so nothing changes in practice. `FOR NO KEY UPDATE`, only on DELETE, would be enough.
- **A redundant scan.** `rate_key_columns_sql` reads the unleased grants twice, as `count` and as `NOT EXISTS`.
- **A terminal run can show `GRANTED`.** A finished run whose slot still counts shows `rateLimit.state: GRANTED` for up to `per`. Say so in `RunRateLimit`'s description.
- **Studio has no wording for the new values.** Its `WAITING` map (`frontend/apps/studio/src/features/runs/display.ts:33`) has no text for `rate_limit` or `concurrency`. It falls back to "Waiting", so nothing breaks; this belongs to Studio's task.

## What holds

**Layers**
- The domain is untouched.
- The application types (`RunRateSlot`, `RatePlace`, `RateLimitKeyView`, `RateLimitKeyPage`, `KeyListing`, `estimated_grant`) import no SQLx or Axum.
- The port grows by two methods (`FlowControlKeyReader::rate_limit_keys`, `rate_place`), and the SQL stays in `infra-postgres`.
- The DTOs exist only in `api-http`. Their `From` impls do real work: they map enums to wire strings, keep the digest out, and apply the label rule.

**`KeyPosition` shared, with a `KeyListing` tag: the right reuse.**
- Both listings use the same total order and the same sealing.
- The tag sits inside the sealed plaintext, so neither listing's cursor can be replayed in the other.
- Existing `k1` cursors keep opening, and the tag length is the same, so `MAX_CURSOR_PLAIN_BYTES` still holds.

**Contract**
- The change only adds: a new path, three new schemas, an optional `Run.rateLimit`, and two values in a response enumeration. 400, 422 and 503 are documented in both documents.
- The SDKs never read `waitingOn`. They also tolerate unknown fields on `Run`: Java turns off `FAIL_ON_UNKNOWN_PROPERTIES`, and Rust uses `deny_unknown_fields` only on IR types.
- Marking the path `x-implemented: '022'` automatically puts the new operation into the tenant battery (`tests/contract/security-multitenancy/rest/battery.rs:598`): another tenant gets 404, a missing token 401.
- The upgrade oracles are bumped, and `scripts/check-upgrade-boundary.py` enforces that.

**Migration**
- It is additive: a nullable column, a backfill, then the CHECK added `NOT VALID` and validated by the next migration, as in 20270205.
- The function stays `SECURITY DEFINER` with a pinned `search_path`, schema-qualified tables and `REVOKE … FROM PUBLIC`. `CREATE OR REPLACE` keeps the owner and the `ENABLE ALWAYS` triggers.
- The `down` restores the previous body byte for byte (as in `20270130000100…up.sql:119-146`) before it drops the column. Nothing is lost, because the column is derived.
- The undo-to-0 test in `tests/integration/platform-foundation/migrations.rs:88` covers the pair.
- Waiters are never updated (`rate_limit_rows_guard`), so keeping the column on insert and delete is complete.
- Because the trigger keeps the column, older writers keep it correct too.

**Hot path**
- No work-budget line moves: the lateral join is inside the same statement and the trigger's statements are nested.
- The CHECK `rate_limit_grants_shape` (`leased_at = granted_at` at run scope) makes structural the claim that no unleased step holds a slot ahead at run scope.

**Operability**
- One `REPEATABLE READ READ ONLY` snapshot per page, `SET LOCAL statement_timeout = '2s'`, at most 201 rows a page, and a timeout reported as a typed 57014.
- `QUEUED_RUNS_SQL` is reused, so `queued_runs_count_errors` still finds exactly one count. The reveal decision goes through `reveals_personal_data`.
- There are no new metric labels and no dependency on the broker.

**Decisions**
- D022-067 records reversible read-model choices and is properly delegated; no ADR is needed.
- Nothing diverges from ADR-0088, ADR-0089 or ADR-0093. The in-force cursor check applies ADR-0093 decision 5.

**Scope**
- The only 022 branches are `feature/022-throughput-control` and `feature/022-sdk-builders`. The `sagat-wt-022-sdk` worktree has no rate listing, and its migrations end at `20270206000100`.
- Neither `sagat-wt-022-sdk` nor `sagat-wt-041` has a migration numbered `20270207*`.

## Fitness summary

| Rule this change relies on | Check | State |
|---|---|---|
| Domain and application free of SQLx and Axum | `check-architecture.py` (layers) | Holds |
| Every implemented operation: another tenant gets 404, a missing token 401 | `security-multitenancy/rest/battery.rs` | Holds; covers the new path automatically |
| No raw key value in any response | `rate_rest.rs` canary; `key_value_free_errors` | Partly: add `routes/rate_limits.rs` to the files it checks (Minor 1) |
| One count of queued runs | `queued_runs_count_errors` | Holds |
| One reveal rule | `reveal_rule_errors` | Holds |
| Upgrade oracles follow the latest migration | `check-upgrade-boundary.py` | Holds |
| Every migration has a `down` and rolls back | `platform-foundation/migrations.rs` (undo to 0) | Holds on an empty schema |
| Runtime role privileges | `runtime_privileges` | Holds (reads only) |
| A rate listing page costs its own keys | none | Missing: EXPLAIN twin of `rest.rs:678` (Majors 1 and 2) |
| `oldest_wait_at = min(enqueued_at)` for rate windows | none (the CHECK covers only "null exactly when nobody waits") | Missing: `key_consistency` row and tamper test (Major 3) |
| No estimate while the run also waits for a permit; `waitingOn: concurrency` | none | Missing: two REST tests (Major 5) |
| Position uses the pass's own eligibility rule | review only | Missing: shared SQL macro (Minor 4) |
| A cursor of one listing is refused by the other, both ways | integration test, concurrency to rate only | Partly: unit tests of the tag (Minor 7) |
| Rate bodies match the canonical schemas | none | Missing: `OpenApi::validate` in `rate_rest.rs` (Minor 8) |
| Cost added by triggers and nested work | none (the budget sees top-level statements only) | Gap by construction: note it in the plan, measure in T022-089 (Minor 6) |
| Component schemas only grow (response enumerations, `Run` fields) | review only (`iceberg-writes/openapi.rs` hashes operations, not schemas) | Existing gap; not introduced here |
