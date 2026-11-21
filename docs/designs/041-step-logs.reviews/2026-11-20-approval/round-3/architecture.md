**Verdict:** design 041 is not ready for approval at `c52b5d37`. Every Blocker and Major from the earlier rounds is now resolved except N-1, which is only partly resolved. Its fix brings in a new Major, and the wait-slot change (D041-134) brings in a second one. Both are small text fixes and neither is a Blocker. I had no shell, so this comes from reading the files, plus the sqlx 0.9.0 source in the local cargo registry (the version `Cargo.lock` pins).

## Earlier findings

| Finding | Status | Where |
|---|---|---|
| A-1 (no backend pid without a statement) | resolved | `docs/designs/041-step-logs.md:1408-1449`, ADR-0097 decision 3b |
| A-2 (G041-018 left open) | resolved | `docs/designs/041-step-logs.md:524-526`, OD-2026-11-20-19 |
| A-3 (SC-041-007 per size) | resolved | D041-135 (`specs/041-step-logs/research.md:146`); the hypergeometric figures check out (20/364, 924/38,760) |
| A-4 (smoke check) | resolved | `docs/designs/041-step-logs.md:2506`, script in T041-5's Files `:2592` |
| A-5 (withdrawn cost and wait lines) | resolved (as at round 2) | — |
| A-6 (drop-reason set) | **resolved** | `failed` is in `DropReason` (`specs/041-step-logs/data-model.md:19`), in the allowed `p_mark_reason` values (`:609`, `:634`), in the drop keys (`:613-615`), in T041-36 (`docs/designs/041-step-logs.md:2799`, `:2848`) and in the split re-run tests (`:2941-2947`). This matches ADR-0095 decision 5 (`docs/adr/ADR-0095-…md:108`). |
| A-7 (ADR-0095 fitness row) | resolved | `docs/designs/041-step-logs.md:2856-2858` |
| N-1 (connection hand-off) | **partly** | Ownership is now specified (`:1414-1418`, `:2807-2828`), but the mechanism as written still cannot be built. See N-2. |

## New findings

**N-2 (Major, fix). `close_on_drop` cannot be cleared.** This is what is left of N-1.
- **Where:** `docs/designs/041-step-logs.md:1416-1418` and `:2815-2816` ("`commit` sends `COMMIT` and clears `close_on_drop` only when it succeeds"); `resolution.md:196-197`.
- **What is wrong:** in `sqlx-core-0.9.0/src/pool/connection.rs` the flag is a private field (`:23`). The only public method, `close_on_drop()` (`:97-99`), sets it to `true`. Nothing sets it back to `false`, and `Drop` (`:200-204`) closes whenever it is set.
- **Consequence:** built literally, every run transaction closes its connection after a successful commit, so each one opens a new backend and pays a new authentication. Or the implementer improvises.
- **Blast radius is understated:** `PgRunTx` is not only a report transaction. It is every run command's transaction: `open_run` at `crates/infra-postgres/src/execution.rs:447` and admission at `:1513`.
- **Missing check:** no listed test fails on that churn. The fault test (`:2948-2951`) only checks that dropped or failed transactions close.
- **Smallest correction:**
  - The owning wrapper keeps an `open` flag that is set at `BEGIN` and cleared only when `COMMIT` or `ROLLBACK` returns Ok.
  - Its own `Drop` calls `close_on_drop()` on the inner `PoolConnection` only while `open` is still set.
  - Say that this applies to every `PgRunTx` command, not only reports.
  - Add one assertion to the fault test: with a pool of 1, two committed transactions in a row run on the same `pg_backend_pid()`.

**N-3 (Major, fix). The wait-slot setting may not reach a server backend behind a transaction pooler.**
- **Where:** `docs/designs/041-step-logs.md:1366-1368`, `specs/041-step-logs/data-model.md:251-254`, T041-5 `:2495-2498`, T041-37 `:3009-3017` and `:3045-3055`, `docs/runbooks/step-logs.md:67-69`.
- **What is wrong:** *Q* reaches SQL only through a custom session setting, `sagat.step_log_wait_slots`, set when a client connection is opened. In transaction mode PgBouncer assigns server backends per transaction and restores only the parameters it tracks (`application_name` is one of them). A custom `sagat.*` setting is not, so on pooled backends `sagat_quota_step_log_wait_slots()` would answer 1 (*Q* = *P* = 1). The pool also already sends its settings through the startup `options` parameter (`crates/infra-postgres/src/pool.rs:59-71`), which PgBouncer refuses or drops unless configured, and the planned `pgbouncer.ini` says nothing about it. I could not run PgBouncer to confirm the behaviour of a given version.
- **Consequence:**
  - The pooler rows of T041-37 judge "at most *P* = 3" and "*Q* = 5". With *Q* = *P* = 1 those pass trivially, while `service_busy` rises. That busy rate is what triggers ADR-0097 option 1c.
  - The runbook's "raising `SAGAT_DB_MAX_CONNECTIONS` raises it" is false behind a pooler.
  - The bound itself fails safe (fewer waits, never more).
- **Smallest correction:**
  - State what happens behind a pooler (*Q* = 1 unless the setting reaches the backend).
  - Have T041-37's pooler rows record `sagat_quota_step_log_wait_slots()` as a carry sees it through the pooler, and judge *P* and *Q* at that value.
  - Have `pgbouncer.ini` say how startup parameters are handled (`ignore_startup_parameters` / `track_extra_parameters`), including the existing `options`.
  - Correct the runbook line.

## Nits

1. **Leftovers of the old top-up-as-refill wording**, which contradict D041-134. T041-5's tests (`docs/designs/041-step-logs.md:2525-2530`) would catch an implementation that followed them, so fix as text:
   - section 2's L4 says the top-up waits up to *b* holding a wait slot (`docs/designs/041-step-logs.md:987-989`);
   - section 2a's table says a top-up `made` writes "each quota lease it reclaims" (`:1259`);
   - "the top-up the refill" (`specs/041-step-logs/data-model.md:247`); the refill's `bounded` parameter (`:286`) now has no caller that passes false;
   - `docs/adr/README.md:39` lists the top-up as a refill;
   - the top-up's `held` answer is listed (ADR-0097 `:135`, design `:1423`) but never defined for a function that reclaims nothing.
2. **T041-36's Files miss two files.** It lists `execution.rs` and `execution/path.rs` (`:2987`), but leaves out:
   - `crates/infra-postgres/src/execution/scoped_events.rs:102`, which takes `&mut Transaction<'_, Postgres>` and is called with `&mut self.tx` (`execution.rs:5536`, `execution/path.rs:315`);
   - `crates/infra-postgres/src/execution/database.rs`, which holds an `impl PgRunTx`.
3. **ADR-0097's header is out of date.** Its amendment note (`docs/adr/ADR-0097-…md:9-13`) does not name D041-134, which changed decisions 1, 3b and 4 of an accepted ADR. The sentence also stops at "cites the row" with no period. Under README rules 2 and 4, it should say that the owner's approval of this run is what accepts the change.
4. **"Default pool of 20" is the chart's value, not the binary's.** The binary's default is 10 (`docs/runbooks/scaling.md:36`, `crates/infra-postgres/src/pool.rs:31`). At 10, *Q* = 2 and *P* = 1, and the connections step-log work never holds are 10 − 5 − 2 = 3. Say so where 20 is called the default (`:1363`, runbook `:46`, `:64-65`).

All round-two Nits are fixed: the privilege list is now given one way, the session-setting exception is stated, README line 37, the warm pin, top-ups taking no slot, and the research.md cells.

Counts: Blocker 0, Major 2, Nit 4