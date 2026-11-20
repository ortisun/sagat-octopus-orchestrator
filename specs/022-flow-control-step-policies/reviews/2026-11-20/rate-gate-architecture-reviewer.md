# Architecture review: feature 022 rate gate

Reviewer: `architecture-reviewer` (read-only, diff and spec only), 2026-11-20, over the uncommitted change later committed
on `feature/022-throughput-control`. Text as returned; resolutions in `rate-gate-responses.md`.

**Verdict:** the layering is clean; the one decision core for both levels and both gates closes G022-012; the lock reasoning behind D022-064 item 5 is sound. Three blockers would turn CI or a suite red, all small. Six Majors should be fixed or explicitly accepted before the rate capability is listed in a release.

## Blockers

- **B1.** `scripts/tests/test_check_architecture.py:966-1325`: a pasted second copy of lines 620-965 and the stale `test_the_rate_gates_feature_of_the_postgres_adapter_belongs_in_dev_dependencies_only`, which fails now that `TEST_ONLY_FEATURES` lost its entry; CI `architecture` (`python3 -m unittest discover -s scripts/tests -v`) goes red. Fix: delete the copied block and the stale test.
- **B2.** The upgrade oracles (`release_upgrade.rs:205`, `release_upgrade_inflight.rs:181`, `release_upgrade_callback_cutover.rs:123`) still assert `20270205000100`; the latest migration is `20270206000100`; `scripts/check-upgrade-boundary.py` exits 1.
- **B3.** The hot-path budget (NFR-022-007, T022-076) moved: `refused_by_ceilings` now runs `existing_keys(Windows, …)` for every round with a rate-limited step and `live_keys(Windows, …)` when a window is new. Run the suite; raise the lines with their reason, or fold the window existence check into the semaphore statement.

## Major

- **M1.** A reset of a rate-limited start is refused with a retryable 503 although the capability is now listed (G022-016): `BINARY_CAPABILITY_MISSING` means "a process that lacks a capability the registry requires", retryable; an SDK retries forever. Either build the refusal now (a slot or a non-retryable typed reason; leave only the copied-event reset rules to T022-086) or refuse before the store with a non-retryable reason recorded as a product limit and a release gate.
- **M2.** `maintenance.rs:1249` `#[doc(hidden)] pub async fn purge_rate_grants_as_of(now, …)` lets a caller choose the clock that shortens grants (ADR-0088 decision 1; rule 11; D022-057's precedent). Gate it behind a test feature, add a call-site rule, or drop it.
- **M3.** No cross-run consistency check covers the rate tables now served: `key_consistency` reads only `concurrency_*`; `verify_start_rate` checks only the run's own start. Extend `key_consistency` (window counters across runs, empty windows, rate waiters of ended runs, unleased grants of steps neither `READY` nor leased) and add a principles row.
- **M4.** ADR-0088's required checks (T022-080 fault suite `rate_limits.rs`, T022-081 lock-order barriers `rate_limit_locks.rs`) do not exist; runbook §6 explains activation without the production warning §1.2 has. Add "Not in production before T022-080, T022-081 and T022-086" and the two smallest barrier tests (a dual-gated admission waits on a held window while already holding the semaphore; `record_rate_leases` completes with the window held).
- **M5.** The purge does not scale: every 300 s on every replica, no shard partitioning, at most 1,280 keys a run against 10,000 live keys by default, no order, a full scan of `rate_limit_grants`, up to 5,000 rows per key under its hot lock, only an `info` log. Drive it by key shards, delete by the indexed range, smaller batch and a time budget, emit `scheduler_actions_total{kind="rate_purge"}` and an oldest-purgeable-age gauge.
- **M6.** `readiness_gate.rs:1496`: a full concurrency buffer on a dual-gated start is reported `RATE_LIMIT_QUEUE_FULL`; `contracts/errors.md:24` defines it for the rate buffer or `queued_runs`. Pass the refusing gate's kind.

## On D022-064 item 5 (no hint update at the lease)

Mostly sound: updating the window from the lease transaction would lock it after grant rows, the reverse of window→grants, and contradict the plan's revised T23; the rest rule generalises the plan's "now plus the shortest period" to each waiter's window. It diverges only from the coordinator's instruction, and research records it. The "exact" claim is wrong: the key's home-shard owner computes `now + p` for a grant leased in the journal but not yet copied (the lease pass runs on the run's shard owner), so the hint is late by the lease-recording lag (within ADR-0088's accepted bound, never early). Say so, or lower the hints after `record_rate_leases` commits in a statement that locks only windows.

## Minor

- **m1.** The lease UPDATE locks many grant rows in no fixed order without `SKIP LOCKED`; two replicas during a takeover can deadlock (retried next tick). Order by primary key with `FOR UPDATE OF u SKIP LOCKED`.
- **m2.** The rate-lease pass emits no `outcome="failed"`; the metric catalog (`docs/runbooks/execution-engine.md:127`) lists none of `rate_lease`, `gate_grant`, `gate_timeout` (G022-005).
- **m3.** `passes.rs:5588-5594`: the new doc attached to `rate_leases` is `gate_grants`'; its "rests until a release sets its hint again" is stale.
- **m4.** `application/src/maintenance.rs:167`: the purge deletes up to `limit` per key, `keys × limit` a call.
- **m5.** `docs/runbooks/flow-control.md:48-50` still says no process lists `flow-control-rate-v1`; §6 should say to finish the rolling upgrade before setting a `rate_limit_*` limit (G1192) and that the rollback of `20270206000000` refuses while one exists.
- **m6.** The down migration's refusal is untested.
- **m7.** Feature 041 (`sagat-wt-041`, `20270201000000_step_log_quotas`) restates the same three lists; whichever lands second drops the other's names. Name 041 in the header; port 041's guard test.
- **m8.** ADR-0088 needs a revision note (decision 6's deletion is by terminal transitions; lease recording takes no fence; "Enforced by").
- **m9.** Per-second limits are silently clamped to 20; warn at publication or show the effective limit in T022-078's run view.
- **m10.** T022-072's evidence requires event, cron and child starts; do not tick it without them. G022-009/T022-102 now applies to rate refusals too.

## Nit

`quotas.rs` `RateLimitLiveKeys` usage counts every key version; the gate only versions in force.

## What holds

Layering (pure domain additions; typed store errors; no empty mappers); transactions (semaphores then windows, ADR-0050 slot 7 then 8b; purge window then grants; a run grant stores `leased_at = granted_at`, which fixes the grant routine's run insert that would have violated the CHECK); lease recording fence-free and sound; compatibility (older processes do not list the rate capability, so activation is refused while any is live; CHECKs NOT VALID then validated; enums only grow); telemetry closed enums, spans carry only the group.

## Fitness summary

Domain free of adapters: holds. A capability is listed only when served: holds. Upgrade oracle at the latest migration: red (B2). Hot path unchanged: likely red (B3). Start lock order and lease recording without a window: no check (T022-081). Rate rows, counters and journals agree across runs: no check (M3). No caller-chosen clock shortens a grant: no check (M2). Restated quota lists name every resource: no check on this tree (m7). Purge boundary: holds through the hook of M2.
