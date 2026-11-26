# Architecture re-read of the third-read fixes of the step-policies design (2026-11-26)

Short re-read of the fixes alone (OD-2026-11-25-2 step 4): commit `1276d5b7` of `feature/022-step-policies`, against
`reviews/2026-11-25/design-third-reads-and-rulings.md` and `design-third-responses.md`, by the architecture reviewer.

Verdict: no Blocker. Two new Majors to write in before slices 10 to 13 build tasks T022-243 and T022-244; they do not block
accepting the ADRs. Rulings 2, 3, 5, 6, 9, 10 and the acceptance met; rulings 1, 4, 7, 8 and 11 partly met.

| Ruling (Majors it answers) | Verdict | Evidence |
|---|---|---|
| 1. Dispatch outcome in `run.finished` (architecture Major 1, correctness Major 1) | partly | ADR-0136:103-107, `plan.md:960` (T8), `data-model.md:781-784`, `:797` (invariant 22), `journal-events.md:60`, `spec.md:622`, `tasks.md:1148`. The journal stays a leaf (the late row is the run's own, explicit `seq`, `crates/infra-postgres/src/execution.rs:5726-5735`); replay equals projection (the reducer ignores the field). Three places still call the dispatch statements "last" (minor 3). |
| 2. One draining cause, a further stop refused `409` (correctness Major 2) | met | ADR-0136:43-46, `plan.md:959` (T32), `errors.md:44`, `journal-events.md:18`, `data-model.md:786`, `spec.md:562`, `tasks.md:1151`; premise holds (`AlreadyStopping`, `crates/domain/src/execution/command.rs:2483`). |
| 3. The overdue pass between pick and lock (correctness Major 3) | met | ADR-0136:51-54, `plan.md:1086`, `spec.md:562`, `tasks.md:1152`; D022-262 says it differently (Major 1). |
| 4. Continue-as-new refused at admission (correctness Major 4) | partly | ADR-0136:119-126, ADR-0137:73-76, `errors.md:45`, `:107`, `:119`, `plan.md:961`, `:1088`, `spec.md:622`, `tasks.md:1153`; resets and reruns under a handler tree not covered (Major 2). |
| 5. A budget deferral is not an attempt | met | ADR-0136:118-119, `plan.md:961`, `:1087`, `data-model.md:788`, `errors.md:120`, `tasks.md:1149`. |
| 6. The producer's locks for a hit (architecture Major 2) | met | ADR-0135:46-59, ADR-0056:106-115, `plan.md:952`, `:964-965`, `data-model.md:790`, `spec.md:460`, `tasks.md:1154`, `:1163-1164`; checked against today's code (fence reads take no lock, `execution.rs:5496-5531`; lost-lease pick none, `maintenance.rs:743`; binding replacement key, then `worker_sessions`, then steps, `bindings.rs:88-255`). The ADR-0050 slot table does not carry the pre-read or the re-check (minor 7). |
| 7. `RunRecovery` (security Majors 1 and 2, architecture Major 3) | partly | ADR-0136:73-76, `plan.md:1019-1030`, `data-model.md:791-794`, `errors.md:42-43`, `openapi-flow-control.yaml:323`, `spec.md:582`, `tasks.md:1150`, `:1162`; the binding check has no lock or slot and the grant as written cannot work (minors 5, 6). |
| 8. Slices that must merge alone (architecture Major 4) | partly | `tasks.md:1156-1158`, ADR-0146:36-38; the order is acyclic (`tasks.md:1012-1014`); older slice bullets not struck (minor 9). |
| 9. ADR-0146 order (architecture Major 5) | met | ADR-0146:33-36 matches `plan.md:968` (T31). |
| 10. The draft-executions lock | met | ADR-0137:44-46, ADR-0050:136-137, `plan.md:964-965`, `data-model.md:796`, `spec.md:714`, `tasks.md:1155`; no cycle (`versioning.rs:283-310`, `identity.rs:252-263`). |
| 11. Leftovers | partly | Struck as listed; others remain (minor 10). |
| Acceptance (D022-271) | met | Status lines of ADR-0135, 0136, 0137, 0146; `docs/adr/README.md:144-147`; ADR-0050:132-137, ADR-0056:57, `:117-119`, ADR-0094:45-47; `tasks.md:899`. |

## New findings

### Major (not blocking the ADRs)

1. **The overdue no-op is tied to the wrong thing.** D022-262 and `design-third-reads-and-rulings.md:22` make
   `stop_run(TimedOut)` on a run in its handler phase a no-op, but `StopReason` has only `Cancelled`, `TimedOut`, `Failed`
   (`command.rs:2521-2529`) and the handler's own timer is also a timeout that must drain (ADR-0136:41-42): built as written, the
   handler timer becomes a no-op and the phase has no bound. ADR-0136:51-54 is right (the pass skips the run). Fix: restate
   D022-262 as a no-op of the overdue pass (a deadline source, not a stop reason); add to T022-243 a domain case that the handler
   timer still drains while the deadline stop is a no-op.
2. **Continue-as-new under a handler tree is not refused for resets and reruns.** `data-model.md:666` makes every reset or rerun
   started by a handler-tree run a handler-tree run (the `RunRecovery` path); ruling 4 lists only the dispatch, a child and a draft
   test, and `errors.md` has no row for the reset or rerun routes, so a recovery rerun of a continue-as-new workflow is admitted
   and then ended at its first continuation (the failure matrix claims "one test per creator", `plan.md:1088`). Fix: refuse such a
   reset or rerun at admission (`422 CONTINUE_AS_NEW_REFUSED`), its row in `errors.md`, a case in T022-244; name the code and
   status the runtime defence ends a run with.

### minor

3. ADR-0136:103, ADR-0050:128 and `data-model.md:777` still call the dispatch statements "last"; say "followed only by the
   `run.finished` row of the run's own journal".
4. Invariant 22 (`data-model.md:797`) compares against the row's state "at the commit", which later changes; restate it as
   "present exactly when a row for this source run exists; `PENDING` exactly when `lineage_first`".
5. The `RunRecovery` binding check has no lock (ADR-0136:73-76, `data-model.md:791-794`); read it `FOR SHARE` in slot 4a after
   memberships, add it to the slot table and a `slot_order.rs` case.
6. `run_recovery_namespaces` grants SELECT only to `sagat_app` yet is written by a route that runs as `sagat_app`
   (`data-model.md:791-794`); state the INSERT and DELETE grants or a security-definer function.
7. The ADR-0050 slot table lags the amendments: slot 5 (`:92`) lacks the pre-read and the producer's membership lock, slot 7
   (`:94`) the re-check, slot 4a (`:91`) the draft-executions key.
8. Task T022-255 is too broad: once ADR-0146 routes a scoped finish to a cacheable root successor the rule would be red on
   `crates/infra-postgres/src/execution/scoped_events.rs:164` (harmless: binding replacement updates only leased or running rows,
   `bindings.rs:245-246`); narrow it to leased or running step rows and `worker_sessions`; give T022-254 and T022-255 a slice
   (slice 7, with T022-210).
9. Older slice bullets not struck: `tasks.md:1046`, `:1106-1108` (T022-225, T022-226 in slice 2), `:1109` (T022-245 in slice 3).
10. Leftovers outside ruling 11: `data-model.md:684` ("`config_revision` is the revision read with the run's definition"), T022-214
    (`tasks.md:963`, `:1037`), T022-227 ("the run deadline during the phase", `tasks.md:1071`).
11. The overdue "skipped" count (`plan.md:1086`) has no closed-vocabulary label in the observability table.
12. Nothing checks the ADR index against the status lines (`scripts/check-agent-config.py:13`, `:215` check only that a Status
    line exists); compare each ADR's first Status word with its row in `docs/adr/README.md`.

### nit

ADR-0136:109-110 ("counts each attempt … and checks the budget"; T9 checks the budget first); ADR-0094:44 still calls ADR-0146
"(proposed)"; nested parentheses in ADR-0056:56-57; `plan.md:1093` "the first cause wins"; the plan's `RunRecovery` "column" is a
separate table (`plan.md:1019-1030`); the stored value of the role is not named (existing values are lowercase,
`20261124000000_database_executor.up.sql:4`); the "Decided by" lines of ADR-0135, 0136, 0137, 0146 still read "accepted once … a
re-review".
