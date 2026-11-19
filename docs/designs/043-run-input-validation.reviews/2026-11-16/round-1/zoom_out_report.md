# egm-review Stage 5 (Initial Zoom Out), round 1

Doc: docs/designs/043-run-input-validation.md
Reviewed commit: f562614874016262dd9812e241f734038d4d70de
Stage reviewer: egm-zoom-out (zoom-out-standard, EGM 0.1.0, tree hash 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9)
Note: the reviewer's output, condensed by the author only in layout; every finding and severity is kept. The reviewer
read files only.

Context sufficiency: passes. Not raised because the owner decided them: the platform's own validator
(OD-2026-11-14-14), `report` mode and its capability (OD-2026-11-14-8), the fair permit queue (ADR-0090 decision 4).
Reuse checked: the classification cache is made generic (`Cache<K, V, E>`); `off_worker` is an 8-line copy, said openly,
acceptable because sharing it needs a new crate edge.

## Major

1. **Technical Plans 3, 4 and 5 and Appendix A are an implementation spec, not architecture** (lines 355-356, 401-600,
   602-658, 1123-1147, the register citations). Pseudo-Rust types, the full regex allowlist in a table cell, the
   unit-by-unit charges, cache weights, lazy DFA sizes, migration timestamps, about 60 line references to `c2e9341f`,
   and a register id in almost every sentence. Fix: move the subset (keyword table, pattern allowlist, pre-check limits)
   to a contract file such as `specs/043-run-input-validation/contracts/input-schema-subset.md`, the charge table and
   Appendix A to `research.md`; symbol names only; Technical Plan 3 as the port, the gate and the type in prose.
2. **Gate wiring relies on a fail-closed default plus a text-scanning lint, where a constructor parameter would let the
   compiler enforce it** (lines 207-209, 418-420, 443-449, 775, 911-913). Fix: `Arc<RunInputGate>` a constructor
   parameter of `RunService`, `Scheduler`, `EventDispatcher`, `ScheduleService`; drop `unwired()` and rule 7; record it in
   ADR-0092 decision 2 (about 66 `::new(` call sites, mostly test support).
3. **The legacy continue-as-new stays a production write path, kept unreachable only by a regex rule and an exemption**
   (lines 193-199, 354-355, 374). Fix: gate the trait method and its implementations behind
   `#[cfg(any(test, feature = "test-support"))]`, or move the tests to the evaluated path and delete it; or add a
   follow-up task if out of scope.

## Nit

4. Part of rule 4 duplicates what the compiler enforces (lines 201-204, 434-439); narrow it to `Default`/`Deserialize`.
5. Task text contradicts the plan: T043-3 "tokio stays a dev-dependency" (902), T043-5 graded disclosure (922-924),
   T043-24 "for the owner role" (1080).
6. Known follow-up work is not tracked as follow-up tasks (G043-052, O043-005, O043-006, G043-053, G043-006).
7. The Overview carries the process history (lines 25-46).
8. A compile panic is treated as transient although a failure memory exists (lines 415, 476-479, 581, 776).
9. The uncached ad hoc path (`shared: false`) may no longer be needed now that a tenant holds at most a quarter of the
   cache (lines 412, 440, 581-583).
10. `recheck` is a third gate entry point that exists only to adjust metric counts (lines 425, 614-617, 689-690).

Counts: Blocker 0, Major 3, Nit 7
