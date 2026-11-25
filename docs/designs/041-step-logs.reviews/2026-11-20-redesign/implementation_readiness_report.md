# EGM stage 9 (implementation readiness) of the 2026-11-19 revision of design 041

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 4c027adffaa889109a4b776c148d0f11263f9130
Reviewer: egm-review
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Counts: Blocker 2, Major 35, Nit 21
Note: stage 9 only, DetailedRange lines 1500-2232 of commit 4c027adf, three goldfish dispatched directly (egm-implementation-readiness, egm-codebase-details, egm-style) with the implementation-readiness standard; recorded condensed by the author. Counts are the sum of the three (readiness Blocker 0, Major 11, Nit 5; codebase details Blocker 2, Major 6, Nit 6; style Blocker 0, Major 18, Nit 10).

## egm-implementation-readiness

- IR-1 Major. The seam names `PgStore` and an input `PgRunTx` lacks. IR-2 Major. The acceptance rule's baseline of gate (3), batch rate of gate (4) and a round not run. IR-3 Major. What value goes into the defaults. IR-4 Major. The carry's API shape and the switch's wiring. IR-5 Major. Which transactions carry, re-run or drop on failure and scoped paths. IR-6 Major. Barrier hooks no task builds. IR-7 Major. Untested carry outcomes; metric owners. IR-8 Major. Removing the seam breaks the driver; missing files. IR-9 Major. T041-8 tests carry behaviour that lands in T041-36. IR-10 Major. T041-4 omits data-model §6's correction. IR-11 Major. T041-9's workloads undefined. IR-12 to IR-16 Nit: the feature's guard; the spike's allowance substitute and backend tag; two owners of T9's `last_stored_xid`; T041-3's baseline removal and the report Debug test; the Verification Plan and ADR-0095 out of step.

## egm-codebase-details

- CD-1 Blocker. CA cannot read through `effective_policy` (Rust) in one SQL statement, and the snapshot predates the lock. CD-2 Blocker. Carry sites and re-runs mapped to the wrong or incomplete code paths (FailStep's and the scoped recoveries; `reject_secret_output` already ends in `finish`). CD-3 Major. Changing three signatures breaks about 50 callers. CD-4 Major. C1's switch and the port's bound have no home. CD-5 Major. `StoreError::StepLogCarry` does not exist. CD-6 Major. `PgStore` does not exist; `plan_batch` is already built on the branch. CD-7 Major. T041-3 and T041-4 describe built work as new. CD-8 Major. The scoped path does not share the stop rule. CD-9 to CD-14 Nit: no T4 hold; private `tx` fields; the L1/L2 slot; stale line references; the `perf-spike` feature's crate; the SPEC section order.

## egm-style

- SY9-1 to SY9-18 Major: T041-6's gate against T041-33/34; owner gates of OD-2026-11-19-1; T041-35's gate scope; "seam" and "hook"; the hook's one case and (g); (g)'s limits; a round not run; "row" with two meanings; the T4 hold; "none" twice; T041-8 tests T041-36's behaviour; four names of the canary; T041-14's renumbering; the Estimate's counts; T041-9's scope; T041-36's files; T041-3's carry behaviour and sentence; T041-5's accessor. SY9-19 to SY9-28 Nit: wording and wrapping.
