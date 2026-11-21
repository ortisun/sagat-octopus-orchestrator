# Security re-review: feature 022 rate gate fixes (code a67ba790, documents a3a2d2d3)

Reviewer: `security-reviewer` (read-only, diff and spec only), 2026-11-20, over commits a3a2d2d3 and a67ba790 of
`feature/022-throughput-control` (diff `d5e0c792..a67ba790`). Text as returned; resolutions in
`rate-gate-rereview-responses.md`.

**Verdict:** No Blocker. The fix for the earlier security Major 1 creates a new Major: aged windows are no longer counted, so `rate_limit_live_keys` can now be bypassed and the per-namespace bound on window rows is gone. The earlier Major 2 and the test-only door are only partly resolved. I could not run anything. Every finding comes from reading the committed code, and each one names the command that would confirm it.

## Status of the earlier Blockers and Majors

| Earlier finding | Status | Evidence |
|---|---|---|
| Security Major 1 (purge lag counted against other tenants) | **Resolved for its scenario** | `readiness_gate.rs:619-627` and `quotas.rs:1370-1378` count a window only while it has a waiter, or a grant that is unleased or leased within `max_period_ms`. A tenant's admission no longer depends on another tenant's purge backlog. Fix (b) of that review (an oldest-first, indexed purge) was not done (G022-018). No test covers it. It also introduces the new Major 1 below. |
| Security Major 2 (lease pass cost) | **Partly** | `maintenance.rs:319` adds `st.attempt >= u.attempt`, so a grant that was never leased no longer costs a journal probe. Each tick still joins every unleased step grant of each owned shard to `steps` and sorts them before `LIMIT` (`:311-331`). See Minor 3. |
| Security minor 4 and architecture Major 2 (test-only door) | **Partly** | `purge_rate_grants_as_of` is still `pub` (`maintenance.rs:1305`). The new call-site rule can be bypassed (Minor 2). |
| Architecture Blocker 1 and correctness Major 1 (checker test file) | Resolved | In the diff, the only hunk in `scripts/tests/test_check_architecture.py` is at 962. Confirm: `python3 -m unittest discover -s scripts/tests -v` |
| Architecture Blocker 2 (upgrade oracles) | Resolved | The three oracles now assert `20270206000100`. Confirm: `python3 scripts/check-upgrade-boundary.py` |
| Architecture Blocker 3 (hot-path budget) | Resolved as claimed (not run) | `work_budget.rs` budgets 60, 44, 48 and 48, each with its reason. Confirm: `cargo test -p sagat-tests --test integration_claim_transition_efficiency work_budget` |
| Architecture Major 1 and correctness Major 5 (reset of a rate-limited start gets a retryable 503) | **Not resolved; deferred behind a release gate** | `readiness_gate.rs:1122`; runbook `:184`. A pending reset keeps its `pending_resets` quota inside the tenant. This is acceptable only while the production gate is honoured. |
| Architecture Major 3 (cross-run consistency of the rate tables) | Open | T022-079 |
| Architecture Major 4 (fault suite, barriers, production warning) | Partly | The warning is at runbook `:184`. `rate_limits.rs` (T022-080) and `rate_limit_locks.rs` (T022-081) do not exist. |
| Architecture Major 5 (purge does not scale) | Partly | `SKIP LOCKED` (`maintenance.rs:1341`) and 500 grants a key (`passes.rs:97`) are in. The scan is still unordered, unindexed, unfair between tenants and has no time budget (`:1312-1319`). See Minor 5. |
| Architecture Major 6 and correctness minor 1 (refusal kind) | Resolved | `start_refused(…, key_of, …)`, covered by a unit test of both dual-gated cases |
| Correctness Major 2 (count instant) | Resolved | `readiness_gate.rs:683` (`$8 = now`) and `gate_queue.rs:201` (`$12 = now`) |
| Correctness Major 3 (rest computed later than the counts) | Resolved | The rest uses the hint the pass was selected by (`maintenance.rs:264-276`). See suspicion 1. |
| Correctness Major 4 (grant through another key's pass) | Resolved | `readiness_gate.rs:751` and `gate_queue.rs:287` call `window_due_after_grant` (`readiness_gate.rs:1436`) |
| Security Nit 7 (usage counts every key version) | **Not fixed, though the responses say it is** | `quotas.rs:1370-1378` has no `key_version` filter, and its own comment admits it. See Nit 6. |

## Major

### Major 1 (new): a window that no longer counts is still treated as existing, so `rate_limit_live_keys` can be bypassed and window rows are unbounded

**Where:**
- `readiness_gate.rs:579-584`: `existing_keys` for windows tests only that the row exists.
- `readiness_gate.rs:521` (step path) and `:1191-1200` (start path): an existing key skips the ceiling check entirely.
- `readiness_gate.rs:619-627`: the ceiling now counts only live windows.

**Scenario:**
- Tenant A has the default ceiling of 10,000 and publishes two definitions on the same group:
  - D1, `rateLimit {limit: 1, per: second, key: input.k}`;
  - D2, the same key with `per: hour`.
- A starts D1 with fresh values of `k`. Each run grant gets `leased_at = granted_at` and stops counting after 1 s.
- The purge keeps the row for at least `max_period_ms + 60 s` (`maintenance.rs:1316`), plus whatever backlog G022-018 adds. So every window lingers at least 60 s while not counted.
- Within that time A starts D2 on the same `k`. The window "exists", so no ceiling check runs, and the window then counts for an hour.
- At R starts a second, A holds about R/2 × 3,600 live hourly windows within an hour, whatever the ceiling. At R = 100 that is 180,000 against a ceiling of 10,000.
- A does not even need to revive windows to grow storage. Churning distinct keys leaves about R × (61 s + purge lag) rows. The purge serves the whole deployment at about 1,280 keys per 300 s per replica, so any tenant above about 4 new keys a second makes rows grow without bound.

**What crosses tenants:**
- The purge candidate query scans all of `rate_limit_grants` 20 times per housekeeping on every replica.
- A's backlog takes purge capacity from every other tenant, because candidates are picked in no order.
- A deployment-wide digest-key rotation cannot end while A's rows under the retiring version stay. Runbook §2 step 3 tells the operator to "wait for the purge".
- In A's own namespace, `live_keys` and the usage query cost grows with every grant row: the `leased_at + make_interval(...) > clock_timestamp()` predicate cannot use `rate_limit_grants_count_idx`.

**Before this fix:** every row counted, so rows per namespace were bounded by the ceiling.

**Fix:**
1. Smallest: put the same liveness predicate in the windows branch of `existing_keys`. Reviving an aged window then counts as a new key and meets the ceiling. This also narrows the key-existence oracle (earlier suspicion 3) to live windows.
2. To bound total rows: add a plain-count backstop (for example, refuse a new window when all rows of the namespace reach k × the ceiling), or make G022-018's indexed, tenant-fair purge a production gate in §6.

**Confirm:** add a test to `rate_gate.rs`:
- set `RateLimitLiveKeys` to 1;
- start a per-second key `a`, wait past 1 s, start key `b` (admitted: correct);
- start an hourly definition on `a` (admitted today without any ceiling check);
- assert that `rate_limit_live_keys` usage is 2, which is over the ceiling.

Run with `cargo test -p sagat-tests --test integration_flow_control_step_policies rate_gate`.

## Minor

### Minor 2: the rule guarding the caller-chosen purge instant can be bypassed, and the door is still public

**Where:** `scripts/check-architecture.py:873-890`; `maintenance.rs:1305`.

The bypasses, each verified against the regex:
- **A function-path reference with no parenthesis** does not match `\bpurge_rate_grants_as_of\s*\(`. For example, `let p = PgRunStore::purge_rate_grants_as_of; p(&store, later, 64, 500)` in `crates/scheduler/src`.
- **Any `self.purge_rate_grants_as_of(` anywhere in `maintenance.rs` is exempt.** The exemption is keyed by file, not by the enclosing function. So a second `pub fn purge_at(&self, now)` wrapper passes. The checker already has `enclosing_function`, which `LEGACY_FORWARDER` uses for this reason.
- **`GATE_ROW_MAINTENANCE` exempts by statement prefix and file** (`:457-461`, `:479`). Any other `DELETE FROM rate_limit_grants g USING rate_limit_windows …` in `maintenance.rs` passes, including one that deletes unleased grants or skips the window lock.

**Impact:** with a future `now`, the function deletes grants that still count, and the prune trigger removes the window. That is over-delivery. Only code inside the workspace can call it; no external caller can.

**Fix:**
- Match the bare identifier (`\bpurge_rate_grants_as_of\b`).
- Exempt only inside `fn purge_rate_grants` by `enclosing_function`, and key the two statement exemptions the same way (`record_rate_leases`, `purge_rate_grants_as_of`).
- Or put the function behind a test-only feature listed in `TEST_ONLY_FEATURES`, the precedent this slice just removed.

**Confirm:** add both bypasses to `ThePurgeAsOfAChosenInstantIsTheTestsOnly` and run `python3 -m unittest scripts/tests/test_check_architecture.py -k Purge`. I expect exit code 0 where 1 is wanted.

### Minor 3: the lease pass still costs a tenant-controlled amount every tick

**Where:** `maintenance.rs:311-331`.

**Scenario:**
- A definition has a rate-limited root step on a queue no worker polls.
- Each key holds up to `limit` granted, unleased steps, across up to `rate_limit_live_keys` keys.
- Every tick, on every shard owner, each of those grants costs a read of `rate_limit_grants_unleased_idx` and a `steps` probe, and all are sorted before `LIMIT $1`.
- The journal probes are gone, but the work is still O(parked grants) per tick.

**Fix:** record the lease in the lease transaction (the earlier review's longer-term fix), or keep a per-grant "probe after" instant so a parked grant is skipped. Record the remaining cost under G022-018.

**Confirm:** `EXPLAIN (ANALYZE, BUFFERS)` of the statement with 10,000 parked grants on one shard.

### Minor 4: the runbook's production gate (section 6) and the rotation advice are incomplete

**Where:** `docs/runbooks/flow-control.md:184-188`, `:206`, `:89-92`.

- **The gate list omits G022-018, which is now the only bound on rows (Major 1), and T022-079.** T022-079 is the safety net for earlier suspicion 1: an unleased step grant of a run that ended outside the reducer is never purged, because the purge requires `leased_at IS NOT NULL` (`maintenance.rs:1316`).
- **"Rate-limit rows drain by time … wait for the purge" is false for a key in continuous use.** `file_keys_with` reuses the row under the retiring digest when it exists (`flow_control.rs:219-224`). A key with traffic gaps shorter than `max_period + 60 s` plus the purge lag never moves to the active digest, so the rotation needs the force of section 3.
- **There is no way to see the purge backlog.** Only a `done` counter exists.
- **Line 206 presents "no longer counts, purged or not" as harmless.**

**Fix:**
- Add G022-018 and T022-079 to the bold gate.
- State the continuous-key case and the force route.
- Give a query for the oldest purgeable lease, or add the gauge from architecture Major 5.

### Minor 5: an all-skipped purge call ends the housekeeping's purge, and the same unordered candidates come back every round

**Where:** `maintenance.rs:1312-1319`, `:1341`; `passes.rs:~4299-4310`.

- `SKIP LOCKED` makes "0 deleted" mean either "nothing to do" or "every candidate was locked", and `n == 0` stops all 20 rounds until the next housekeeping (300 s).
- With no `ORDER BY`, the same first 64 keys are picked each round. A tenant with 64 keys each holding more than 500 purgeable grants takes every round: 64 per-second keys at the clamp of 20 per second build 6,000 grants per key per housekeeping.

**Fix:** return the deleted and skipped counts and continue while some were skipped; pick candidates in tenant round-robin order (G022-018).

## Nit

### Nit 6: the usage query still counts every key version

`quotas.rs:1370-1378` has no `key_version = ANY(in force)` filter, so usage can show more than the gate enforces after a forced retirement. Either add the filter or say so in `contracts/`.

### Nit 7: the lease pass's hint lowering runs one statement per window

`maintenance.rs:367-390` sends one autocommit statement per recorded window, even when the window has no waiters. Filter with `waiters > 0` through a join in the first statement, or issue one `UPDATE … FROM unnest(...)` sorted by primary key; sorted locking cannot deadlock.

## Suspicions (not confirmed)

1. **Catch-up amplification after a scheduler lag** (`maintenance.rs:264-276`). An unleased grant rests its key at `$5 + p`, where `$5` is the stale hint the pass was selected by. After a lag L, a per-second key with an unleased grant and a waiter needs about L / 1 s visits before its hint reaches the present. Before the fix, one visit was enough. This could stretch a lagging grant pass for every tenant. Confirm: set one key's hint an hour back, call `grant_gates` repeatedly, and count the passes until `next_eligible_at >= now`.
2. **The lease pass now waits on window row locks** (`maintenance.rs:367-390`), with no `lock_timeout`, inside the scheduler tick. Before, it locked only grant rows. A decision transaction that holds a window lock delays the tick on every tenant's shard. Confirm: a barrier test that holds the window lock and runs `record_rate_leases`.
3. **The purge candidate query may scan the whole table on every call.** `SELECT DISTINCT … LIMIT 64` with an expression predicate likely plans as a HashAggregate over all of `rate_limit_grants`. Confirm with `EXPLAIN ANALYZE` on 1 million rows.
4. **Earlier suspicions 1 and 4 are unchanged.** `docs/owner-decisions.md` does not mention `rate_limit_live_keys`.

## Classes checked with no finding

- **Tenant boundary.**
  - Every new or changed statement carries tenant, namespace, pool, scope, group and digest in its joins: `window_due_after_grant` (`readiness_gate.rs:1436-1468`), the lease-pass hint lowering, `live_keys`, the usage query, the per-key purge transaction and its `DELETE … USING`, and `verify_start_rate`.
  - The purge candidate query and the lease pass are global by design; each per-key action binds the tenant it read.
  - No new API surface, so the 404 indistinguishability is unchanged.
- **Privileges of `sagat_app`** (`20270130000100:249-255`).
  - The lease `UPDATE` uses the `leased_at` column grant.
  - The hint updates use the `next_eligible_at` and `updated_at` grants.
  - `FOR UPDATE SKIP LOCKED` on windows is satisfied by the column `UPDATE` grant.
  - The grant delete and the `USING` read are granted.
  - The count and prune triggers are SECURITY DEFINER with `search_path = pg_catalog, pg_temp`.
  - The new migrations change only CHECK constraints on quota tables, `NOT VALID` then validated, and the down migration refuses while a rate-limit limit or ceiling is retained.
  - Confirm with `SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_flow_control_step_policies rate_gate`.
- **Failing closed.**
  - No key ring, or the switch off, means not served (`served`).
  - Collections and loops are refused; so is a reset of a rate-limited start.
  - Mismatched keys produce an internal error.
  - A zero effective limit or timeout is refused before anything is filed.
  - Errors in the lease pass and the purge only keep slots counting longer.
- **Secrets and labels.** Metric labels are closed values (`kind`, `outcome`, `scope`). Spans carry only the group. Logs carry counts and the classified store error. Refusals use static limit names.
- **Injection.** All SQL is static with bound values; `make_interval` takes only integer columns.
- **Races.** The lease `UPDATE` touches `leased_at IS NULL` rows and the purge `IS NOT NULL` rows, so they are disjoint. The guard trigger allows one NULL-to-value change. The purge locks the window first. The hint lowering runs after the commit, one window per statement, so no transaction holds a grant row while it waits for a window.

## Test gaps

- **Aged, unpurged windows are not counted.** Deleting the `EXISTS` clause at `readiness_gate.rs:622-626` leaves every test green today.
- **The ceiling bypass of Major 1.**
- **Cross-tenant isolation of the passes:** the purge or lease pass over tenant A leaves tenant B's rows and hints unchanged.
- **The rate suite as `sagat_app`:** no run is recorded in the responses.
- **Purge with a held window:** it skips that key and returns without blocking (a barrier, not a sleep).
- **The two checker bypasses of Minor 2.**
- **The down migration's refusal** (architecture minor 6, still open).
