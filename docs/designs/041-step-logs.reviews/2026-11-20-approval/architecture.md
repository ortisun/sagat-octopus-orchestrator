# Architecture review of design 041's revision (approval run, first round)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: ff8033cd933d0c6adf972706eb1bb38407bdb7c6
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a fresh `architecture-reviewer`, read-only, given the diff of the design and of its companion documents since the approved commit f9d4a65f, on the owner's approval path "Full review, capped" (OD-2026-11-20-17). Saved verbatim.

I reviewed the design as it stands at `ff8033cd` against the constitution, ADR-0027, -0084, -0085, -0095, -0097, `docs/adr/README.md` and `docs/engineering/architecture-principles.md`. I found one Blocker and six Majors. One limit on what I checked: I had no shell, so I could not run `git branch -a`. For duplicated work I read the branch heads under the repository's `.git/worktrees` instead, and found no other branch working on 041.

**What holds**
- **Layers:** the domain holds `plan_batch`, `Carry` and `CarryPlan`, with no SQL, framework or `serde_json`. `StepLogCarryTx` has one method and no rule. The plan is copied into plpgsql, which ADR-0095 accepts and a property test guards.
- **Decisions:** the ADR-0027 "Amendment of the Amendment" breaks README rule 2, but the owner granted that exception in writing (OD-2026-11-20-16). ADR-0095 and ADR-0097 are recorded as accepted (OD-2026-11-20-15), and both list alternatives, consequences and "Enforced by".
- **Compatibility:** protocol minor 1.7 is not released (`docs/releases/` has only 0.1.0, 0.2.0, a historical 0.4.0 and `unreleased.md`), so changing it in place is allowed. The new column, table and functions are additive. The down migrations refuse while lines exist rather than destroy data.

## Blocker

**A-1. The store cannot learn which database session (backend pid) a pooled connection is without running a statement.**
- **Where:** `docs/designs/041-step-logs.md:1255-1263` ("The kept room"), section 2b's session-affinity bullet, ADR-0097 decision 3b, and the affinity-miss metric in `specs/041-step-logs/data-model.md` §7.
- **What is wrong:** the design says the store "takes the pid from the call's returned writer and keeps it with the connection, so it reads it with no statement of its own". The pinned sqlx 0.9 makes this impossible:
  - `sqlx-postgres-0.9.0/src/connection/mod.rs:51` keeps `process_id` private.
  - The only public accessor there is `server_version_num` (`:89`).
  - Pooled connections carry no per-connection state, and their address moves inside the pool.
  - The prototype never tested this cache: T041-35's fourth run leased every connection at pool fill instead.
- **Consequence:** the build has two options, and both break what was accepted:
  - Add a `SELECT pg_backend_pid()` round trip to every carrying report. Then the "warm" equality pinned by T041-36 is wrong.
  - Key the kept room by tenant only. With 20 connections per pool the guess is usually wrong, so most carries would refill under the run lock. That is the regime the third run failed on, and it is not what OD-2026-11-20-13 accepted. Decision 3b would quietly lose its effect.
- **Recommendation:** specify a mechanism that works with sqlx and count its cost. The smallest one: after a carrying report commits, top up the lease on that same connection when the room the carry returned is short. That needs no pid, because the answer came from that connection. The first carry of each backend falls back, or pays a counted `SELECT pg_backend_pid()` on acquire. Then:
  - restate the warm and cold equalities and ADR-0097 decision 3b's wording;
  - add a T041-36 test that a warm carrying report runs no statement outside its budget.
- Recommended disposition: fix. The cost the owner accepted assumes a mechanism that cannot be built as written.

## Major

**A-2. A security condition is still open, but the design says no owner decision is open.**
- **Where:** `docs/designs/041-step-logs.md:460` says "None is open since 2026-11-20". But the Constitution Check, principle V (`:1933`), says G041-018 is "put to the owner with this revision's approval". The Risk row at `:1899` and `specs/041-step-logs/research.md:157` both say it is open.
- **Consequence:** approving now would pass principle V with a condition the owner never accepted. G041-018 is the case where a holder of `run:logs:read` without a namespace's PII grant can watch `tenant_step_log_bytes` usage move.
- **Recommendation:** add an O-question under "Open" (show `used` only with the grant in every namespace, or accept the residual in writing). Record the answer before approval.
- Recommended disposition: fix. Only the owner can accept a principle V condition, and the design's own text says it is pending.

**A-3. SC-041-007 is "settled" with no threshold, and T041-9's release gate now conflicts with it.**
- **Where:** SC-041-007 (`:374-386`) still states 90% of the rate and p99 within +10%, and records the failed 1 × 16 figures as accepted. T041-9 (`:3071-3072`) says "a failure of SC-041-007 stops the release".
- **Consequence:**
  - At 1 × 16, T041-9 either fails by construction or is waived with no rule.
  - Nothing catches a regression below the accepted 82.0%.
  - The accepted figures leave out what T041-35 never measured: the ensure transaction and the scoped report path.
- **Recommendation:** restate SC-041-007 per size. Keep 8 × 128 as written. At 1 × 16, set a floor at the fourth run's figures with a declared tolerance (rate of (a) and (f), (2b) excess at most +8.6 ms plus the tolerance). Have T041-9 judge by that floor.
- Recommended disposition: fix. A criterion that cannot fail does not guard the hot path.

**A-4. T041-5's per-replica `application_name` breaks the compose smoke check.**
- **Where:** `docs/designs/041-step-logs.md:2545` changes `pool.rs:70`. `scripts/smoke-compose.sh:81` checks the least-privilege role by `application_name = 'sagat-octopus-orchestrator'`. T041-5's Files list does not include that script.
- **Consequence:** the compose smoke step of the release gate fails, because `API_USERS` comes back empty.
- **Recommendation:** keep `sagat-octopus-orchestrator` as a prefix (for example `sagat-octopus-orchestrator/<replica>`), match it with `LIKE` in the smoke script, and add the script to T041-5's Files.
- Recommended disposition: fix. A one-line change that otherwise turns up only at release.

**A-5. Text that still states withdrawn cost and wait rules.**
- **Where:**
  - `:402` "six for a long step";
  - `:448` "the round trips (five for a short step)";
  - `specs/041-step-logs/research.md:161` (G041-022) "a short step's carry adds 5 [round trips], about 10%", against `:959` "one round trip … about 2%";
  - `:3168` (Verification Plan) "a carry's refill answers `contended` at once", against section 2b's wait of up to 200 ms that ends `tenant_quota_busy`, and T041-36's own test;
  - `:341` "no free lease with room" (wording from the withdrawn shared pool);
  - `docs/adr/README.md:37` "five or six statements … dropped by one re-run … (would supersede …)".
- **Consequence:** a test written from the Verification Plan would assert the withdrawn `NOWAIT` behaviour. G041-022 overstates its risk about five times.
- **Recommendation:** correct each line to ADR-0095 decisions 2 and 5 and ADR-0097 decision 4.
- Recommended disposition: fix. The Verification Plan is what the tests are written from.

**A-6. The drop-reason metric's label set is not closed over what the carry emits.**
- **Where:** `specs/041-step-logs/data-model.md:419` limits `step_log_lines_dropped_total{reason}` to `worker_gap`, `attempt_records`, `attempt_bytes`, `tenant_quota`, `tenant_quota_busy` and `out_of_order`. The carry returns drops for `invalid`, `secret` and `content_fallback` too (`:534-535`), and section 2a counts records as `failed`. T041-36 says it counts carry lines in the drop metrics.
- **Consequence:** the implementer must either invent label values or leave drops uncounted. The vocabulary is no longer one the design fixes (principle VI).
- **Recommendation:** add the four reasons to the enum and to §7, or state that those drops are counted only through `step_log_carries_total{outcome}`.
- Recommended disposition: fix. A small edit that keeps the telemetry rule checkable.

**A-7. ADR-0095's promised fitness row in `architecture-principles.md` is not planned.**
- **Where:** ADR-0095's "Enforced by" says each check comes "with its row in `docs/engineering/architecture-principles.md`". The only row is row 31, which covers ADR-0097 alone and still says "Proposed". T041-36's Files list leaves out `architecture-principles.md`.
- **Consequence:** these rules have tests in tasks but no registered fitness row (README rule 3, AGENTS rule 21):
  - a report reaches the step-log tables only through `StepLogCarryTx`;
  - a carrying report costs exactly one more statement;
  - a carrying report never waits for a step-log lock;
  - the SQL plan equals `plan_batch`.
- **Recommendation:** have T041-36 add one row naming `work_budget.rs`'s carry equalities, the restated source-text test, the lock-order and held-lock tests, and the plan property test. Fix row 31's status.
- Recommended disposition: fix. The ADR promises it and it costs one line.

## Nit

**A-8.** ADR-0097's header (`docs/adr/ADR-0097-…md:9`) still has the pre-acceptance clause ("puts this ADR to the owner … once design 041's re-review has no Blocker or Major") under `Status: Accepted`. ADR-0095's Evidence still says "The figures that decide whether this ADR is put for acceptance are T041-35's". Note that the owner accepted both even though T041-35 did not pass (D041-115).

**A-9.** `docs/engineering/architecture-principles.md:31` says "(ADR-0097, Proposed)".

**A-10.** The ensure's `invalid` bound (`data-model.md:266`) uses the attempt ceilings' maxima, about 16 MiB plus 12.8 MB. A valid carry plans at most about 16 KiB plus 65 × 128 bytes. ADR-0097 decision 3b says "larger than any valid carry's". Tighten it to a constant derived from the carry cap, so a store defect cannot strand tens of MB in one lease.

**A-11.** The adapter mechanics are underspecified:
- the kept-room cache has no stated bound or eviction;
- it applies a 9-minute staleness rule while the text says "the adapter holds no rule";
- the port method and trait that begin a carrying report, and where `CarryRoom` lives (an application port), are not named in T041-36 or its Files;
- acquire, ensure and begin together need the cancel-safety that `begin_safely` (`crates/infra-postgres/src/pool.rs:123-132`) gives today.

**A-12.** Stale Risk rows: `:1884` "(the completion p99 within 10%)" leaves out the 2 ms allowance; `:1888` says "against the unchanged gates"; `:1893` says G041-025 "closes only with none", but it is closed. The Verification Plan's gap list (`:3190-3200`) stops at G041-017, while principle VIII cites G041-001 to G041-027.

**A-13.** OD-2026-11-20-13 and D041-123 say T041-35 is ticked, but it is `[ ]`. Add the explanation T041-33 carries: `check-design-docs.py`'s rule R9 lets a task ticked since `develop` stand only in an Approved doc, so the approval pull request ticks it.

**A-14.** Section 9 has the store write a structured warning naming the tenant for every `tenant_quota_busy` carry. While the usage row is stuck this is unbounded log volume. Rate-limit it per tenant and state the limit.

**A-15.** The cold-path count "+1 (the ensure's call), +1 transaction" depends on whether the ensure is one autocommit statement or `BEGIN`/`SELECT`/`COMMIT`. State which, since T041-36 pins it as an equality.

Counts: Blocker 1, Major 6, Nit 8
