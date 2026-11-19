# Architecture review of design 043 (2026-11-15)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: ce2e10e2eab95102e658d370dcef1fcd4a48faeb
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Findings are numbered
across the report so that an acceptance can cite `architecture.md#<n>`. The reviewer read files only and ran nothing.

## Blocker

1. **The doc and ADR-0090 say `sagat-application` has no runtime; the code and the architecture rule allow one, and no
   new ADR records it.** Doc lines 181, 397-401, 695, 711, 797, 805-806 and ADR-0090 ("Enforced by", Consequences) say
   tokio is only a dev-dependency and the sixth rule fails on any normal `tokio`. `crates/application/Cargo.toml` has
   `tokio` (`rt`) as a normal dependency, `collection_source.rs` calls `spawn_blocking`, and
   `scripts/check-architecture.py` exempts `rt` and searches only `run_input.rs` (D043-150, D043-154). An accepted ADR
   is not edited in substance; diverging needs a new ADR. Fix: a new ADR refining ADR-0090 (G043-052 as the way back),
   the six passages corrected, the rule tightened to "tokio only in `collection_source.rs` and test modules" with a
   negative fixture.

## Major

2. **The legacy continue-as-new rule is weaker than the doc says** (lines 173-174, 314-315, 333). The check exempts any
   call inside any function named `continue_as_new`, in any crate (D043-122). Fix: key the exemption by (file,
   function), add a negative fixture, write the exemption into FR-043-016.
3. **Services default to a gate that refuses every schema as busy, and the doc does not say so** (lines 340, 383-392,
   418-420). `RunService::new`, the event and pass schedulers default to `RunInputGate::unwired()` (D043-120); a
   composition that forgets `.with_run_inputs` makes every start of a version with a schema a retryable 503 that engine
   creators retry forever, and no app test asserts the wiring. Fix: describe `unwired()`, and either gate it behind
   `test-support` with the gate as a constructor argument, or add a composition-root test in each app.
4. **OD-2026-11-15-3 is not in the doc, and Technical Plan 9 promises more than it covers** (lines 263-264, 649-654,
   887-902). T043-14 has no `Owner-gate: OD-2026-11-15-3` and does not list the baseline fixtures; line 652 still
   promises the 422 text on schedule writes, trigger-now and publication, which the decision does not cover (D043-135).
5. **FR-043-015 promises what the contract plan cannot carry** (lines 167-170, 649-659, 903-908). G043-046: the REST
   `Run` has no run-level failure code and Technical Plan 9 adds none. Fix: an additive optional `Run` field read from
   the run's stop in section 9, T043-14 and T043-15 (six SDK views, Studio types, a conformance case), or narrow the FR
   with the owner's written acceptance.
6. **The doc does not describe what phases 1 and 2 built, and contradicts itself.** The amendments block (lines 624-634)
   sits on top of the text it changes: the `oneOf` bound (lines 470-471 against D043-101); the repeat field policy
   (lines 103-106, 546-548 against D043-121, D043-144, D043-148); "no creator reads the database for the schema" (line
   351 against D043-125, D043-126); row 14's time value (lines 561-562 against D043-125); the backfill pause (lines
   123-124, 325 against D043-134); one migration (lines 557-559, 586-587 against D043-131, D043-145, D043-153); the
   tenant share missing from the cache paragraph (D043-105); the gate sketch missing `check_schema`, `recheck`,
   `SchemaRefusal::Interrupted` (D043-106, D043-127, D043-147); section 7 missing the continuation metrics and the new
   `trigger_events_total` outcomes (D043-129, D043-133). Fix: fold D043-100 to D043-155 into the sections they change.
7. **ADR-0090 has not absorbed later changes to its decisions 5 and 6**: the name limit `INPUT_SCHEMA_NAME_TOO_LONG`
   (D043-102, also a narrowing under principle IV), the tenant share (D043-105), the `oneOf` bound (D043-101); its
   Evidence still cites D043-001 to D043-077.

## Nit

8. Header `Date` is 2026-11-14 though the text changed on 2026-11-15; "phase 1/2" is never defined; say once that phases
   1 and 2 were built before approval under OD-2026-11-14-15.
9. Stale register ranges (lines 41-42).
10. FR-043-012 is not quite true in `report` mode: a schema over 64 KiB or 32 deep is refused `CLASSIFICATION_TOO_COMPLEX`
    in both modes (G043-020).
11. The rollback paragraph (lines 669-670) omits the down migration's refusals on the capability and the requirement
    history, and that binaries can roll back without it.
12. A transient gate refusal on an event is recorded as "storage unavailable; will retry" (`crates/scheduler/src/events.rs`).
13. `docs/engineering/architecture-principles.md` has no row for the six design-043 rules.
14. `InputSchema::new` is public and does not check the digest the cache key trusts.
15. `INPUT_INVALID` is already a worker step failure code (`sdks/SPEC.md`); note the two meanings in `errors.md`.
16. The `INSERT INTO runs` rule reads only string literals with the unqualified name; add a qualified-name fixture and a
    scan of `migrations/*.up.sql`.

Counts: Blocker 1, Major 6, Nit 9
