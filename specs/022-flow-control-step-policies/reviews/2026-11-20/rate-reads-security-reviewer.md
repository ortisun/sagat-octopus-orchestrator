# Security review: task T022-078 (the rate gate's reads)

Reviewer: `security-reviewer` (read-only, diff and spec only), 2026-11-20, over the local commit after d08bb79b on
`feature/022-throughput-control` (diff `d08bb79b..HEAD` without the generated Studio types). Text as returned;
resolutions in `rate-reads-responses.md`.

**Verdict:** I found no Blocker. One Major should be fixed before merge: for every caller with `run:read`, `getRun` returns `estimatedGrantAt` as another run's grant instant plus the window, to the microsecond, with or without the reveal grant. That links the runs of one key, which is what D022-060 truncates instants to prevent. The tenant boundary, cursor sealing, the split between the two cursor kinds, the SECURITY DEFINER trigger and the down migration hold. I verified all findings by reading the code; I ran nothing.

## Major

**Major 1: `estimatedGrantAt` gives a caller without the reveal grant another run's exact instant, which links runs of one key.**
- Where:
  - `crates/application/src/runs.rs:2366-2382` sets the place with no reveal check.
  - `crates/infra-postgres/src/flow_control.rs:983` (`kth`) supplies the value.
  - `crates/api-http/src/dto_flow_control.rs`, `From<&RunRateSlot>`, serializes it whole.
- Why it holds: the `rate_limit_grants_shape` CHECK forces `leased_at = granted_at` for RUN scope (`20270130000100_flow_control_rate_limits.up.sql:72`). So `estimatedGrantAt` equals the k-th holder's `granted_at` plus `per`, and `Run.rateLimit.since` of that holder is the same `granted_at`, shown whole on `listRuns`. `rate_rest.rs::a_run_shows_its_rate_slot_and_a_waiting_run_its_place` asserts exactly this equality. It reads as alice, but no branch changes the value for any other caller.
- Scenario: user `olivia` has `run:read` in `payments` but no reveal grant. She calls `GET /runs/W` and gets `estimatedGrantAt = E`. She calls `GET /runs` and finds the run H whose `rateLimit.since` is exactly `E - 1h`. Now she knows H and W share one key value (for example the same `accountId`, the same customer). The hidden label and the minute truncation of `oldestWaitSince` and `nextSlotAt` exist to stop exactly this. The operations code says so at `operations.rs:686-689`, and D022-067 item 6 says instants derived from a run's own instants are truncated.
- This is new compared with `Run.concurrency`: a permit only ever shows the run's own `since`, never an instant of another run.
- Smallest fix: in `get`, after `reveal_permit_labels`, check `slot.label().is_none()`:
  - recommended: set `place.estimated_grant_at = None` for that caller;
  - or, to match D022-060, apply `to_the_minute`. That only narrows the link, because the holder's `since` stays whole on `listRuns`.
  - Record the choice in D022-067 and the contract.
- Confirm: add a test that reads `/runs/{first}` as `olivia` in `rate_rest.rs` and asserts `estimatedGrantAt != granted_us(holder) + HOUR_US`. Then run `cargo test -p sagat-tests --test integration_flow_control_step_policies rate_rest:: -- --nocapture`. It fails today.

## Minor

**Minor 1: `position` tells a reader without the reveal grant how many runs of the same key are ahead.**
- Where: `runs.rs:2366-2382`, `RATE_PLACE_SQL` `ahead` (`flow_control.rs:962` ff.).
- Scenario: `olivia` reads `position` for each WAITING run of group `accounts`. Runs at the same position are on different keys; runs at strictly increasing positions, in `since` order, can be on one key. Their deadlines and pause state also move the number. With `listRuns`'s whole `since` values, she can partly rebuild which runs share a key.
- This is the G022-014 class of linking, which is already accepted, but `getRun` (`run:read`) now offers it, not only the `ops:read` listing. The concurrency permit shows nothing about other runs.
- Fix: list `getRun` `position` as a surface of G022-014 / T022-111 in `research.md`. Or return null for a caller without reveal, if the owner prefers that to the product value.

**Minor 2: a page's cost is not bounded the way the plan says. `nextSlotAt` scans every grant of the key, aged-out ones included.**
- Where: `flow_control.rs:920-926`. `g.leased_at + make_interval(...) > now()` cannot use an index range, so it walks the whole key prefix of `rate_limit_grants_count_idx` once per distinct waiter period (up to 3). The `DISTINCT x.period_ms` subquery also scans every waiter of the key (up to 10,000).
- Grant rows per key are bounded by the purge, not by the limit. A per-minute key at 6,000 a minute keeps about 12,000 rows, and G022-018's backlog adds more.
- Scenario: a namespace with 201 such keys, each with a waiter and no unleased grant. Each `ops:read` page reads millions of rows until the 2 s timeout gives a 503. That is 2 s of shared-database CPU per request, bounded only by `api_requests_per_second`.
- This contradicts the plan's "at most the hard ceiling of its longest window (10,000)".
- Fix: make the predicate index-usable for each period: `(SELECT min(g.leased_at) FROM rate_limit_grants g WHERE <key> AND g.leased_at > now() - make_interval(secs => p.period_ms/1000.0)) + make_interval(...)`, then take the min over the periods. That is one index probe per period. Get the periods with three `EXISTS` probes instead of `DISTINCT` over all waiters.
- Confirm: `EXPLAIN (ANALYZE, BUFFERS)` of `RATE_KEY_LISTING_SQL` on a key seeded with 20,000 aged grants and one waiter.

**Minor 3: `RATE_PLACE_SQL` runs on the `getRun` path with no statement timeout.**
- Where: `flow_control.rs:810-825` uses `fetch_optional(self.pool())` with no `SET LOCAL statement_timeout`.
- Scenario: a run at the back of a 10,000-waiter key. Each `getRun` counts 10,000 waiters, each with an index lookup on `runs`, counts up to 10,000 grants and runs an `OFFSET` scan. An SDK polling such runs multiplies that per poll, on the database every tenant shares.
- Fix: run it in a read-only transaction with `LISTING_TIMEOUT_SQL`, as the listing does. On a timeout, return no place rather than failing the read.

## Nit

1. `docs/runbooks/api-rate-admission.md:60`: the charged-route inventory (the closed `ApiOperation` contract) has no row for `GET /namespaces/{namespaceId}/rate-limit-keys` → `OpsRead`. The code classifies it correctly; the document does not.
2. `docs/reference/api.md` has no `listRateLimitKeys` row. Confirm with `cargo test -p sagat-tests --test contract_developer_experience` (I expect a diff failure).
3. `routes/rate_limits.rs` calls `concurrency_cursor_in_force` for a rate cursor. It works because the check is generic over versions, but the name invites a later wrong split. Rename it `key_cursor_in_force`.
4. `rate_rest.rs`, in `the_namespace_lists_its_rate_keys_with_the_concurrency_listings_protections`: the "cursor of the concurrency listing" block fetches an empty concurrency page and never uses it. It reads as coverage it does not give.
5. `Run.rateLimit.limit` is the effective limit, so it shows a `run:read` caller the namespace's `rate_limit_per_*` policy whenever that is lower than the declaration. It is low value, but it is a quota disclosure without `ops:read`; note it in the contract or accept it.

## Suspicions (not verified)

1. **The trigger's new `PERFORM ... FOR UPDATE`:** if any writer of `rate_limit_waiters` runs at REPEATABLE READ or SERIALIZABLE, the oldest wait it recomputes is read from the transaction snapshot, not a fresh one, and the lock can raise serialization failures. That is availability and correctness, not security. Every gate path I saw locks the window `FOR UPDATE` first (`flow_control.rs:71`), so the lock mode adds no new conflict with the FK `KEY SHARE` locks there. Confirm with `rg -n "REPEATABLE READ|SERIALIZABLE" crates/infra-postgres/src` and check that none of those transactions deletes or inserts rate waiters.
2. **Other bodies carrying `Run.rateLimit`:** besides `getRun` and `listRuns`, it reaches `RunBody` from the start response (`routes/execution.rs:515`, through `known_start` → `store.get`, which never reveals the label) and from `database_executions.rs`. The label fails closed and no place is set there. I did not check the stream or subscription payloads. Confirm with `rg -n "rate_limit|RunRecord" crates/application/src/subscriptions* crates/api-http/src/routes/stream*`.
3. **Token debit ordering (inherited from the concurrency listing, not new):** the runbook says cross-tenant probes consume no token. The 400 for a retired cursor comes after `admit`, so it debits one. Same as the concurrency listing.

## Classes checked with no finding

- **Tenant boundary:**
  - `RATE_KEY_LISTING_SQL` and `..._OF_GROUP_SQL` filter `w.tenant_id = $1 AND w.namespace_id = $2`, and every subquery correlates on `w.tenant_id` and `w.namespace_id`.
  - In `RATE_PLACE_SQL`, `me` is filtered on `$1` and `$2`, and `counted`, `ahead` (including the `runs` join on `tenant_id`) and `kth` all correlate on `me.tenant_id` and `me.namespace_id`.
  - The run lateral correlates on `runs.tenant_id`, and `QUEUED_RUNS_SQL` is scoped to the tenant and namespace.
  - `rate_place` is only called after `store.get(tenant, id)` found the run, so another tenant's run is the existing 404 path.
  - The service calls `require_namespace`, which gives an audited 404, before the store.
- **Refusal paths:** a bad or foreign cursor (wrong tag, wrong namespace or tenant) and a group mismatch are 400s decided before admission, from the caller's own tenant and the path namespace, so they are the same whether the namespace exists or not. A bad group is a 422 before admission. A retired version is a 400 after admission. Without `ops:read` the caller gets a 403 from `admit` and again in the service. The error texts are constants.
- **Cursor sealing and separation:** the `k1`/`r1` tag sits inside the authenticated, sealed plaintext, and `decode` checks it against the `KeyListing` it was called for. Concurrency cursors issued before this change were `k1` and stay valid. The concurrency-to-rate refusal is tested.
- **Labels and truncation:** the label is set only from `reveals_personal_data`. Without it, `oldestWaitSince` and `nextSlotAt` are cut to the minute. `RunRateSlot` keeps its digest private and its `Debug` omits it, and `KeyPosition`'s `Debug` omits the digest too. No new log, metric or span carries a key or an identifier. The canary is asserted absent from every body in `rate_rest.rs`.
- **Least privilege:**
  - `sagat_app` has table-level SELECT on `rate_limit_windows`, so it can read the new column.
  - Its INSERT and UPDATE grants are per column and do not include `oldest_wait_at`, so only the trigger writes it.
  - The listing and `RATE_PLACE_SQL` need only SELECT on windows, grants, waiters and runs, all granted; neither takes a row lock.
- **SECURITY DEFINER trigger:** `SET search_path = pg_catalog, pg_temp` keeps `pg_temp` last, the table references are qualified with `public.`, and only built-ins are called. `REVOKE ALL ... FROM PUBLIC` is re-run after `CREATE OR REPLACE`, which keeps the owner. The CHECK holds after every row trigger (`LEAST` ignores NULL on insert; NULL when the counter reaches 0).
- **Migration:** `ADD COLUMN` takes ACCESS EXCLUSIVE, so the backfill and the function swap are atomic against writers. The down migration replaces the function before it drops the CHECK and the column, its text matches the original function exactly, and the indexes are dropped.
- **Injection and parsing:** every value is bound. The text cast of `$4` only ever receives the literals `infinity` and `-infinity`. The group is validated by `ResolvedGroup::parse`. The limit is clamped in the route (1 to 200) and again in the service. The query uses `deny_unknown_fields`. The cursor's microsecond value comes from authenticated plaintext. There is no regex or JSON input and no egress.
- **Races:** the reveal check is read without a lock, as accepted for concurrency because a label is a pseudonym. A cursor replayed after a version is retired is refused by the in-force check. Replaying a cursor after losing the reveal grant gives nothing, because the instants inside are sealed.

## Test gaps (each is a test to add, then run with `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_flow_control_step_policies rate_rest::`)

1. `getRun` as a caller without the reveal grant: `keyLabel` null, plus whatever is decided for `estimatedGrantAt` and `position` (Major 1, Minor 1).
2. A namespace of another tenant on `rate-limit-keys` is a 404 with the same body as a missing one, and is audited. `rest.rs:386` covers this for the concurrency listing only.
3. Without `ops:read` → 403. A rate cursor sealed for another namespace, or under a retired or forced version → 400. A rate cursor of group A used in a group-B listing → 400. A rate cursor given to `concurrency-keys` → 400 (the reverse direction is untested).
4. The 503 when the ring lacks the version in force and a next page exists.
5. A unit test in `flow_control_keys.rs` that an `r1` encoding is refused by `decode(.., Concurrency)` and the reverse. `a_position_it_did_not_write_is_refused` covers `k1` only.
6. Running the suite as the runtime role. Nothing shows `rate_rest` passed under `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
7. A cost test or `EXPLAIN` for a listing page over a key with many aged grants (Minor 2).
8. The trigger's `oldest_wait_at` under one statement that deletes several waiters of one key and inserts others (the CHECK holds, and the value equals `min(enqueued_at)`), plus `migrate` then `undo` with live waiters.
