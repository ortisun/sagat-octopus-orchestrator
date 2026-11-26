# Stage 6 (Comprehension) review of design 054 (2026-11-26)

Doc: docs/designs/054-buffered-signals.md
Reviewed-commit: 745a16afa36c55475f17e0e072502a6e8fb82cea
Reviewer: egm-review, stage 6 (the egm-comprehension lens, dispatched directly)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author from the reviewer's output; every finding and its severity is kept. The author's answer
follows each finding.

Counts: Blocker 0, Major 5, Nit 8

The reviewer restated the proposal correctly (an opt-in inbox of the run, one consumer per signal, the 4 MiB ledger, the
lifecycle, the fleet requirement and its floor).

## Major

1. **Whether `maxBytes` applies to a live delivery is not said**, so a large signal could be accepted when a wait exists and
   refused when none does. *Answer: fixed:* `maxBytes` applies to every signal of a declared name, live or buffered (Requirement
   054-004, Requirement 054-005, Technical Plan 3 step 2).
2. **The entry time is the deadline less `timeoutMs`, but whether `timeoutMs` is required is not said.** *Answer: fixed:* every
   `waitEvent` must declare it (the IR schema's `required`), stated in Requirement 054-004.
3. **The live tie-break (lower node id) and the in-transaction consumption (the first wait the planner enters) may disagree.**
   *Answer: fixed:* they are one order; the planner plans root steps in node-id order (`start_pending` over `state.steps()`,
   "ordered by node id"), stated in Requirement 054-004 and Technical Plan 12.
4. **The number of migrations and where the hold function lives conflict** across Technical Plans 2 and 15, Tasks 054-4 and
   054-18, the slices and the Constitution Check. *Answer: fixed:* one migration of this design plus the shared prerequisite
   (Task 054-18) only if designs 044 and 045 have not landed it; Technical Plan 2 keeps only the comparison guard; Task 054-18 is
   its own slice (2a) and pull request; the Constitution Check says so.
5. **Terms used but not defined** (open holder, binding-then-run order, path transaction, closed gate, point, prefix, reapply,
   the reset rules, run segment, recorded no-op). *Answer: fixed:* each has a line in Terms.

## Nit

6. "Partial" is a way to end in Technical Plan 8 but not in Requirement 054-011. *Answer: fixed:* added, with its meaning.
7. `Engine::resolve` and `Engine::run` both named. *Answer: fixed:* the loop of `Engine::run`.
8. `signal.moved` with a count of zero; "none declared" unreachable for a graph run. *Answer: fixed:* journaled when any moved; the
   test of undeclared names is a code run's continuation under another version.
9. "Four rows" in feature 018's registry against three new events. *Answer: fixed:* three new rows and one revised.
10. The register's heading of Decision 054-009 still says required. *Answer: fixed:* the heading carries the revision.
11. "No read MUST return" reads as no obligation. *Answer: fixed:* "A read MUST NOT return".
12. The metric outcomes do not map to every refusal. *Answer: fixed:* each refusal reason has its outcome.
13. AGENTS.md rule numbers cited without their subject. *Answer: fixed:* rule 9 (the race between timeout and signal) and rule 18
    (barriers, never sleeps; tests first) are quoted; both numbers are right.
