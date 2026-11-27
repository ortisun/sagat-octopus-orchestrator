# Fourth security re-read of design 042 (2026-11-27)

Doc: docs/designs/042-stream-operators.md
Reviewed-commit: d83fa162f4052600845c6fab509f419bd4e8247e
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 1, Nit 4

## Previous re-read

1. **Fixed.** `operator_held_room(tenant)` is an owner-owned definer function: no row lock, pinned `search_path`, revoked from `PUBLIC`, executable by `sagat_app`. It is counted in the five (970-981, NFR-042-002 at 1364-1368). `sagat_app` gets no privilege on `operator_held_deltas` (970-971, 1368). Step 5 uses the function (1011-1016) and is run as the runtime role (T042-13 at 1482-1483). A new snapshot question about it is finding 4.
2. **Fixed.** A held-bytes refusal at the seal or the flush, or a commit-time content refusal, is now a counted `quota_wait`: the window stays `OPEN` with a back-off (289, 307-313, 1045-1050, 1076-1077, T042-24 at 1563-1571). This fix creates a hold-and-wait problem on the tenant pool; see finding 1.
3. **Fixed.** `SEAL_EXPIRED` is retryable back to `OPEN` (290, 293-295, 1049-1050, 1079-1080, 1170, 1187). `OPEN` → `REFUSED` is in the state list (941-943) and in T042-58 (1713-1714). Gaps in how the retry works are finding 3.
4. **Fixed.** The commit order is content row first, then the held counter, through `zz_operator_held_flush`, with a barrier test (962-970, 1058-1059, T042-13 at 1480-1484, ADR-0050 line 103).
5. **Fixed.** When the reservation updates no row, the key row is read again, and the `maxKeys - 1` barrier test is added (994-999, T042-15 at 1510-1512).
6. **Fixed in the text.** Marks are written as upserts (1000-1001), and the logging settings are cited (1001-1004, T042-40 at 1618-1621). Nothing applies or checks those settings; see finding 5.
7. **Fixed.** Gap 042-018 is widened to Viewers who see run timing and to readers of the tenant's usage (380-385, 595-603; research.md G042-018 at line 128). The delivery retry's `202` uses the shape without reveal (1171). The approval sentence is reworded (650-651).
8. **Fixed.** The level is a property of the version's `Input` carrier (439-443), tested after the window is purged (T042-53 at 1671-1673). This can be built as described: carriers are keyed by version (`crates/application/src/classification.rs:126-131`), and an unmarked location is already `pii` (`crates/domain/src/classification.rs:42`).

## Findings

1. Major — A `flatMap` seal now waits for held room while still holding its members' bytes, so the tenant pool can lock itself for about 8 days
   - **Where:** design 289-290, 307-313, 933-935 (member values are deleted only when the last index's run starts), 1045-1050, 347 (final windows are purged 7 days after `final_at`), 332-335, 427 (an index input needs only the summary and element `i`).
   - **What:** Checked in the text.
     - A Developer in namespace A of a tenant fills the 1 GiB `operator_held_bytes` pool with about 4,096 `OPEN` windows of their own `window` + `flatMap` chain, where `select` returns roughly the envelope's elements.
     - At its deadline, each window's seal needs room for `fan_out_values` on top of its members' values, which stay held. With the pool full, every seal gets `quota_wait`. The room each seal waits for is held by the waiting windows themselves.
     - After 24 hours each window becomes `SEAL_EXPIRED` and keeps its values until the purge 7 days after `final_at`.
     - For about 8 days, with no further traffic, every element in any namespace of the tenant that adds held bytes ends `FAILED`/`QUOTA_EXCEEDED`, and namespace B's `flatMap` windows are stuck as well.
     - Organic load reaches the same state with no attacker: a pool filled by `flatMap` windows can never seal any of them.
     - Under the previous text, a refused window released its bytes at its deadline. Without `flatMap`, an attacker can hold the pool for at most a window's `maxWaitMs` (24 hours).
   - **Why:** This is a temporary, shared condition that any publisher of the tenant can cause, and it now sustains itself and outlasts its cause. Reread-3 finding 2 was meant to stop exactly that kind of cross-namespace harm.
   - **Correction:**
     - In the transaction of a successful `flatMap` seal, set the members' `value` to `NULL`. Neither the start nor the input of FR-042-032 needs them, and the fingerprint uses `(source, event_id)`.
     - Check only the net held delta. `operator_held_room` already nets the pending deltas when the deletion comes before the read. A `select` that does not enlarge its envelope then never waits.
     - State the hold-and-wait that remains for a `select` that enlarges its envelope. Alert on a sustained `operator_window_seal_total{outcome="quota_wait"}`.
     - Add to T042-24 a test of a pool filled entirely by `OPEN` `flatMap` windows with an identity `select`: they all seal.

2. Nit — Step 5 makes a tenant content-quota shortage a final `FAILED`, which contradicts FR-042-021
   - **Where:** design 1011-1018; FR-042-021 at 335-338; FR-042-031 table at 397; 1009 (a count or `onFull` seal at the apply).
   - **What:** Checked in the text and against the migrations.
     - FR-042-021 says a content refusal is "the dispatcher's wait for room … not a final outcome", and FR-042-031 lists only the held-bytes quota as final.
     - Step 5 checks content through `operator_held_room` and records `FAILED`/`QUOTA_EXCEEDED` when content would be passed. So a tenant near its content quota turns chained deliveries in every namespace into final failures. Only the narrow commit-time race gets the promised wait.
     - Content enforcement is on: `crates/infra-postgres/migrations/20261026000000_activate_durable_byte_quotas.up.sql:5`.
     - Step 5 also refuses an element that completes a `count` window of a `flatMap` chain when only that window's fan-out lacks room. The pass's own seal of the same window would wait.
     - The text of the room function also leaves out `content_reserved_bytes`, which the flush counts (`20270210000000_step_logs.up.sql:848-853`).
   - **Correction:**
     - In step 5, keep only the held-bytes part final. A content shortfall at step 5 rolls back as the dispatcher's wait, the same as the commit-time refusal.
     - A count or `onFull` seal at the apply that lacks room only for the fan-out keeps the element as a member and leaves the window due, for the pass's waiting seal.
     - Have the room function include reserved bytes.

3. Nit — The `SEAL_EXPIRED` retry has no column for its new bound, and section 4 still deletes values on that refusal
   - **Where:** design 905 (the columns: `start_expires_at` only), 293-295, 1049-1050 and 1079-1080 ("a new 24-hour seal bound from the retry"), 933-935, 1035-1041.
   - **What:** Checked in the text.
     - The expiry rule is "`OPEN` 24 hours after `D`", and `D` does not change on a retry. With no stored bound, the next pass refuses the retried window again at once.
     - Section 4 deletes held values on any refusal except `START_EXPIRED`. An implementer who follows it empties a `SEAL_EXPIRED` window, so its retry seals members without values, and the start stops on the missing values until `START_EXPIRED`.
     - The seal selection (1035-1041) does not filter on `next_attempt_at`, so the back-off of 311 can be skipped and `select` and the validator then run on every tick.
   - **Correction:**
     - Add `seal_expires_at`, set to `D + 24h` and reset by the retry, and state the expiry rule against it.
     - At 933-935, write "except a `START_EXPIRED` or `SEAL_EXPIRED` refusal".
     - Add `next_attempt_at <= now()` to the seal's selection.

4. Nit — The held-room read must take its snapshot after the advisory lock, and the statement budget pushes toward one statement
   - **Where:** design 1011-1016, 972-975, 1022-1024 ("the held-bytes check" counted as one statement), NFR-042-001 at 1359-1361 (at most 8 statements); precedent `crates/infra-postgres/src/quotas.rs:31-47` (the lock is its own statement).
   - **What:** Suspected from PostgreSQL's READ COMMITTED behaviour; not run.
     - Writing `SELECT pg_advisory_xact_lock(…), operator_held_room(…)` as one statement, or calling a `STABLE` function, reads with a snapshot taken before the lock wait ended. It therefore misses a concurrent adder that committed while this one waited.
     - Two applies, or a seal with a `fan_out_values` of up to about 25 MiB, can then both pass, and the quota is overshot by up to one addition per concurrent adder.
     - Two statements stay safe, but they make the common case 9 statements with the evaluation's read, above NFR-042-001's 8.
   - **Correction:** Have `operator_held_room` take the same advisory key itself (`quota:operator_held_bytes:<tenant>`) before it reads, declared `VOLATILE`. That is one statement, and its inner query gets a snapshot after the lock. Add a barrier test with two adders at the quota boundary.

5. Nit — "No digest reaches the server log" depends on settings that no shipped artifact applies or checks
   - **Where:** design 1001-1004, T042-40 at 1618-1621; `docs/runbooks/step-logs.md:129-132`; `infra/docker/postgres-init/01-runtime-role.sh:16`.
   - **What:** Verified: the three settings appear nowhere outside the runbook, and the compose init script creates `sagat_app` without them.
     - `log_error_verbosity` is (to my knowledge) superuser-only, so the runtime role cannot set it for itself.
     - A `CHECK` or `NOT NULL` violation on the operator tables would therefore still write `Failing row contains (…\x<digest>…)` on the stack this repository ships.
   - **Correction:**
     - Add the three `ALTER ROLE sagat_app SET …` lines to `01-runtime-role.sh` and to the Helm notes.
     - Add a privilege-audit assertion that runs `SHOW log_error_verbosity` as `sagat_app`, run with `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
     - Word 1001-1004 as a deployment requirement that is checked.

No finding in the other classes I checked:
- **Tenant boundary:** the uniform `404` and the probe audit on chains, windows and both retries, including the `SEAL_EXPIRED` retry.
- **Permissions:** the permissions of the new acts.
- **Metric labels:** `quota_wait` and the wider `expired` are closed values.
- **`operator_held_room` privileges:** its tenant argument exposes nothing `sagat_app` cannot already read. It can `SELECT` the counter, and has `SELECT` on `tenant_retained_usage` (`20261024000000_durable_quotas.up.sql:564-566`). Pending deltas are read only for the current transaction.
- **Commit order:** every `*_quota_flush` is a per-row `DEFERRABLE INITIALLY DEFERRED` trigger on `INSERT OR UPDATE OR DELETE` (lines 446-559). The first one to fire drains every delta and takes the content-policy fence before the usage row (`20270210000000_step_logs.up.sql:782-865`). `zz_` sorts after `operator_*_quota_flush`, so the stated order holds.
- **Last-slot re-read:** correct under READ COMMITTED.
- **Mark upsert:** it runs only under the key lock, after a read of the mark.
- **Gap 042-018:** consistent between the design and the register.
- **Injection, regular expressions, SSRF and egress:** nothing new.

Gaps 042-017 and 042-018 are stated accurately and are the owner's to accept; they are not findings.
