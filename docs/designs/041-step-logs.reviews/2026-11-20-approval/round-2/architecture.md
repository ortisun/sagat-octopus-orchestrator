Re-read of design 041 at `4d6fe4cc` (architecture, approval run, second round)

I read the files only; I had no shell. Besides the design documents I checked the sqlx 0.9.0 sources in the local cargo registry, `crates/infra-postgres/src/{pool.rs,execution.rs}` and `scripts/smoke-compose.sh`.

## First-round Blocker and Majors

**A-1 (Blocker): resolved.** The store no longer needs to know which backend a connection is.
- The ensure and the kept room are gone. The carry returns `path`, `room` and `top_up`, all computed in SQL (design section 2a, lines 1126-1130; data-model.md §8a, lines 600-606).
- The quota lease stays keyed by `pg_backend_pid()` inside SQL, and the top-up runs on the report's own connection (design section 2b, lines 1379-1391; ADR-0097 decision 3b, lines 120-142).
- The warm, warm-and-low and cold pins are restated (design lines 1258-1263). T041-36 tests that a warm report runs nothing outside its budget and that the top-up runs on the same backend (lines 2792-2799 and 2838-2846).
- One thing is left, and it is a different defect: the connection hand-off T041-36 describes cannot be built the way it is written. I raise it as N-1 below, a new Major.

**A-2: resolved.** OD-2026-11-20-19 and D041-133 close G041-018.
- "Open" now says no question is open (design line 501).
- Principle V cites the closure (line 2056), and so does the risk row (line 2022).
- T041-4 has the test.

**A-3: resolved.** SC-041-007 now has a criterion per size (design lines 397-431).
- 8 × 128 keeps the original gates. 1 × 16 has a floor at the worst valid round, with each tolerance derived from that round.
- T041-9 judges by that floor and names its figures (lines 3182-3185).

**A-4: resolved.** The per-replica name keeps the prefix, as `sagat-octopus-orchestrator/<instance id>`. The smoke script matches it with `LIKE`, and `scripts/smoke-compose.sh` is in T041-5's Files (design lines 2449-2455 and 2530). The name is 27 + 36 = 63 bytes, which just fits PostgreSQL's 63-byte limit.

**A-5: resolved for the lines I named.**
- The Goals (lines 446-452) and Open Questions (lines 482-493) are restated.
- G041-022 now says about 2% (research.md line 171).
- The Verification Plan row is restated (design line 3258).
- `docs/adr/README.md:37` is fixed.
- One stale index line on ADR-0097 is left; see Nit 3.

**A-6: partly resolved (Major still open).** There is now one `DropReason` enum, used for the metric label and the carry's drop keys (data-model.md §1 line 19, §7 lines 467-471). But the list of mark-only reasons the carry function accepts is written two ways, and the version the implementer follows contradicts ADR-0095 decision 5:
- **With `failed`:** design section 2a (lines 1122-1123 and 1217-1221), D041-128, data-model.md §8a (lines 590 and 614) and the Verification Plan (line 3257). After a guard's raise the report is re-run with a mark-only carry whose reason is `failed`, and the stream keeps its final mark.
- **Without `failed`:**
  - data-model.md §1 line 19 gives the subset as `invalid`, `secret`, `content_fallback`.
  - T041-36 (lines 2733-2734) has the function refuse any reason other than those three, and its test calls them "the three" (line 2867).
  - T041-36's own test (lines 2855-2858) expects a guard's raise to re-run the report "with no carry … no final mark".
- **Consequence:** a function built from T041-36 raises on `failed`. The re-run's call then fails and the report runs again with no carry, so the final mark is lost after every guard raise. T041-36's test would pass on that broken behaviour, so the task quietly departs from accepted ADR-0095 decision 5.
- **Smallest fix:** add `failed` in data-model.md §1 line 19, in T041-36 lines 2734 and 2867, and in the `dropped_by_server` comment of §8a (lines 594-596). Split the test at line 2855 in two: a guard's raise re-runs with a mark-only `failed` carry and the final mark is kept; a failing carry statement re-runs with none and no final mark is kept.

**A-7: resolved.** T041-36 adds ADR-0095's row to `docs/engineering/architecture-principles.md`, naming its checks, and the file is in its Files (lines 2778-2780 and 2899).

## New findings introduced by the fixes

**N-1 (Major, disposition: fix). The report transaction cannot keep its connection after commit the way T041-36 says.**
- **Where:** design section 2b, lines 1382-1385; T041-36, lines 2742-2747 ("`PgRunTx` and `PgPathExecutionTx` own their `PoolConnection` and begin on it by `begin_safely` … hand the connection to a spawned task"); ADR-0097 decision 3b, lines 125-127.
- **Why it cannot be built as written:**
  - Both report transactions hold a `Transaction<'static, Postgres>` (`crates/infra-postgres/src/execution.rs:3825` and `:942`).
  - `begin_safely` only returns such a transaction from the pool (`crates/infra-postgres/src/pool.rs:115-132`).
  - In sqlx 0.9.0, `Transaction::commit(mut self)` consumes the transaction (sqlx-core `src/transaction.rs:120`). There is no way to get the connection back; on commit it goes straight back to the pool.
  - A struct that owns a `PoolConnection` and a transaction borrowing it would refer to itself, which Rust does not allow safely.
- **What building it really takes:** the store sends `BEGIN` and `COMMIT` itself on a `PoolConnection` it owns, and gives up sqlx's rollback-on-drop guard.
- **Consequence:** returning a connection to the pool does not roll back an open transaction; it only pings the connection (sqlx-core `src/pool/connection.rs:275-324`). So a naive build hands a connection with an open or aborted transaction to the next request, which can be another tenant's. Every helper that takes `&mut Transaction<'static, Postgres>` in `execution.rs` would also have to change signature. This also brings back first-round Nit A-11's cancel-safety point, which the resolution marked moot.
- **Smallest fix:** state the construction in T041-36 and section 2b:
  1. The transactions hold a `PoolConnection<Postgres>`.
  2. A `begin_safely` variant acquires the connection, sends `BEGIN` inside its spawned task, and calls `close_on_drop()` on it.
  3. `commit` sends `COMMIT` and clears `close_on_drop` only on success. Then, if `top_up` is true, it moves the connection into the top-up task.
  4. The helpers take `&mut PgConnection`.

  Add a test, beside `tests/fault/events-webhooks-messaging/cancellation.rs`, that a report future dropped between `BEGIN` and `COMMIT`, or a failed `COMMIT`, closes the connection instead of returning it to the pool inside a transaction. The cost is unchanged, because sqlx already sends `BEGIN` as a statement.
- The other way out would be for the carry to return `pg_backend_pid()` and the top-up to take it on any connection. That contradicts OD-2026-11-20-18's "no backend id is needed in Rust", so it needs the owner.

## Nits (new)

1. **The privilege list is given two ways.** ADR-0097 decision 1 (lines 96-101) and design section 3 (lines 1553-1555) list eight functions the runtime role may execute, including the two constants. data-model.md §3b (lines 244-246), T041-5 (lines 2496-2499) and the Verification Plan (line 3283) say "the six", and say the audit checks exactly that list. The two constants are also called `SECURITY INVOKER` (data-model.md lines 248-251) while the decision lists them among definer functions. Fix: one list of eight, and say which two are invoker functions.
2. **"No session setting is read" is no longer true.** The wait-slot key reads `current_setting('application_name')` (data-model.md lines 256 and 261-262). Yet design line 1356 and T041-5 line 2500 say no session setting is read, and D041-125's reason ("nothing a caller cannot change") does not hold for `application_name`. Fix: name the exception (the slot key only; the bound protects against load, not against the runtime role) and narrow the bypass test to match.
3. **`docs/adr/README.md:39` still describes ADR-0097 with the ensure** ("room the store makes before the report … only by that ensure … (would supersede"). The same goes for ADR-0097 option 1's title, "given room before the report" (line 215).
4. **The warm pin's observation cannot be made as written.** "`pg_stat_statements` of its backend" (design lines 2798-2799) does not work, because `pg_stat_statements` has no per-backend view. State the observation instead, for example `pg_stat_activity` for that pid after a barrier, plus the top-up query's call count unchanged.
5. **Top-ups compete with carry fallbacks for the seven wait slots.** Near the limit a top-up follows nearly every carry (design lines 1474-1475), so top-ups can hold slots and push fallbacks into `service_busy`. Have T041-37 record slot holders by caller, or state why this trade is acceptable.
6. **The top-up has no span or log in section 9** (lines 1968-1969); only a counter. Principle VI asks for telemetry on it.
7. **G041-018's "Closed by" cell still begins "Open; …"** (`specs/041-step-logs/research.md:167`), with "Closed 2026-11-20" appended after it. G041-021, G041-022 and G041-026 read the same way. A reader or a checker can take them for open gaps.
8. **The risk row "Each logging step adds transactions" (design line 2007) gives only the rate far from the limit** (one top-up per about 300 carries). Near the limit there is one top-up transaction after nearly every carry.
9. **T041-9's fan-out workload (c) has no threshold of its own.** Its gates (1) and (2b) are recorded (lines 3176-3177), but the 1 × 16 floor was derived for workload (a), and the Open Questions say a failure "there" stops the release (line 492). State which threshold (c) is judged by.

## Blocking

N-1 and the remainder of A-6 should be fixed before T041-36 is built. Neither breaks a cost the owner accepted, so I do not rate either as a Blocker.

Counts: Blocker 0, Major 2, Nit 9