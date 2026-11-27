# egm-zoom-out on design 052, full round of 2026-11-26

Doc: docs/designs/052-hot-path-dispatch.md
Reviewed-commit: 2348509a1eddabf7d7f44658fab074b7861e6f9d
Note: the reviewer's output, condensed by the author; every finding and severity is kept. The reviewer read files only
and ran nothing.

## Major

1. The switch and rollback machinery (`ready_queue_switch`, four commands, a gated backfill, a runbook step) serves a
   mixed-release state that cannot exist in 0.3.0: every 0.3.0 migration runs in the stopped upgrade, and a rollback
   never crosses it (design 053). Fix: backfill in the 0.3.0 migration; an unconditional trigger; one runtime setting
   for the queue claim.
2. The no-schema remedies of design 038's research were never weighed: a bound on consecutive skipped-as-locked probes
   (`claim()` walks the whole window of 64), and a start offset from a hash of the session; the 58 probes per lease come
   from a closed loop that spins. Fix: build and measure those first; the queue only if still needed.
3. The queue is keyed per step while the probe locks per run (`RUN_FOR_CLAIM`, `SKIP LOCKED`): in the fan-out shape 15
   of 16 pollers skip the same run, so "fan-out at most 2" does not follow. Fix: key by (tenant, run, activity).
4. The candidate filters on `develop` are not carried over: the eligible window then a bounded `INELIGIBLE_SAMPLE` of 4
   (ADR-0075), and the scan's sample of pool-refused steps kept for the claim's audit; ChildRoot possession changes
   without a step's status changing. Fix: the queue replaces only the eligible window; the samples stay from today's
   scan; both orders in the differential test.
5. Hints on the re-arm paths (unpause, a breaker, a quota released, a binding added) add an outbox write to the hot path
   (a quota is released on every completion under one) for a case `idle_retry` already bounds. Fix: wake only on the
   existing `task.ready` hint and in-process commits in 0.3.0.
6. The next step in a completion takes minor 1.9, which `sdks/SPEC.md` gives to design 045 (1.10 to 1.12 to draft 019);
   minors are cumulative, so the six SDKs need the procedures before it; and the costliest remedy is built in six SDKs
   before it is measured. Fix: "the next free minor when its contract merges", an owner gate on the minors before it,
   the server and one SDK first, the other five a follow-up if kept.
7. The CI gate depends on design 051's unbuilt driver, two scenarios of twelve runs per gated pull request. Fix: gate on
   the in-process transitions scenario in 0.3.0; the wire run a follow-up after Task 051-2.

## Nit

1. Open Question 1's rationale rests on a benchmark that will not run in 0.3.0; justify by the on and off gains on the
   free runner, and measure Success criterion 052-001 under an open loop too.
2. The review-history table belongs in the reviews directory, and Task 052-1 is a process step.
3. Task 052-11 edits feature 019 from inside 052; record it as a gap of 019.
4. Completeness checked three times (repair counter, teardown pass, `verify_run`); drop the teardown pass.
5. "Shares its harness with delivery acknowledgement's throughput gate" names no CI job (decision D006-057 was a one-time
   measurement).
6. Stale references in the ADRs (Task 051-12, Success criterion 051-005; 1.1 against 2 probes per lease).

Counts: Blocker 0, Major 7, Nit 6
