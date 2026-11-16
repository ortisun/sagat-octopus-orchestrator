# ADR-0092: The application names a runtime only for the blob-source parse, and the run-input rules refined in the build

**Status:** Proposed (for the owner)
**Decided by:** proposed by an agent on 2026-11-16 from D043-150 and D043-154 (the coordinator's technical call of 2026-11-15) and the review run of design 043 of 2026-11-15; to be accepted or refused by the owner.
**Refines / supersedes:** refines ADR-0090 (accepted 2026-11-14, OD-2026-11-14-15): its "Enforced by" line and its Consequences on the runtime of `sagat-application`, and decisions 5, 8 and 10. Supersedes nothing.
**Enforced by:** `scripts/check-architecture.py`: `application_runtime_errors` (`tokio` in `sagat-application` is at most a dependency with no default features and no feature but `rt`), `run_input_runtime_errors` (a `tokio` token in `crates/application/src/**` outside `collection_source.rs` and test modules fails), `gate_wiring_errors` (a composition root that builds a creator without `.with_run_inputs` fails), with their negative fixtures in `scripts/tests/test_check_architecture.py` (`TheApplicationHoldsNoRuntime`, `test_a_composition_root_wires_the_gate_into_every_creator_it_builds`); `the_heaviest_schema_the_precheck_admits_fits_a_tenant_share_and_is_kept` and `a_name_past_1_kib_is_refused_and_long_names_are_charged_by_their_length` (`crates/infra-schema/tests/run_input.rs`); `report_mode_audits_a_start_that_an_open_holder_answers` (`integration_schedules_bulk_operations`).

## Context

ADR-0090 says that `crates/application` holds no runtime: tokio is only its dev-dependency, the gate of design 043 is
plain async code over the `SchemaValidator` port, and the permits, the wait, `block_in_place` and the backstop live in
the adapter `crates/infra-schema` (D043-071). A rule of `scripts/check-architecture.py` was to fail on any normal
`tokio` dependency of `sagat-application` (D043-077).

Design 042 (collection operators), merged before design 043 was built, parses the array of a blob source in a blocking
task: `crates/application/src/collection_source.rs` calls `tokio::task::spawn_blocking`, and `sagat-application`
gained `tokio` with no default features and the `rt` feature alone. The two merged designs disagree (G043-051). On
2026-11-15 the coordinator kept both by narrowing the rule to `rt` alone (D043-150, D043-154), and the review run of
design 043 of that day found that the narrowed rule allowed `tokio` in any file of the crate and that no ADR recorded
the divergence from an accepted one (architecture review, Blocker 1; AGENTS.md rule 2).

The same review run found three places where the build refined ADR-0090 beyond its words: a name limit that makes more
schemas unusable (decision 5), repeats and trigger-now that always answer `/` (decision 8), and the audit of a reported
start that an open holder answers, where no run is created (decision 10). An accepted ADR is not edited in substance
(`docs/adr/README.md`), so they are recorded here.

## Decision

1. `sagat-application` MAY depend on `tokio` only with no default features and no feature but `rt`, and only
   `crates/application/src/collection_source.rs` MAY name `tokio` outside test modules, for design 042's blocking parse
   of a blob source's array. Every other module of the crate, the gate of design 043 (`run_input.rs`) among them, MUST
   NOT name a runtime. The permits, the wait, the off-worker step and the backstop of design 043 stay in
   `crates/infra-schema` (ADR-0090 decision 4 is unchanged).
2. Every composition root under `apps/*/src` MUST give each `RunService`, `Scheduler`, `EventDispatcher` and
   `ScheduleService` it builds the run-input gate (`.with_run_inputs`) in its builder chain. A service built without one
   holds `RunInputGate::unwired()`, which refuses every start of a version with an `inputSchema` as transient (D043-120):
   safe, never an unchecked admission, but an engine creator would retry it forever.
3. Refining ADR-0090 decision 5: a name of `properties` or `required` longer than 1 KiB also makes a schema unusable
   (`INPUT_SCHEMA_NAME_TOO_LONG`, D043-102). The lower bound of the pre-check counts the cheapest `oneOf` branch plus two
   units for each other branch (D043-101). The compiled form of every schema the pre-check admits MUST weigh no more than
   one tenant's share of the run-input cache (a quarter of its entries and bytes, D043-105), so it is compiled once and
   kept: the lazy DFA caches a pattern's weight counts are 16 KiB per direction (security review of 2026-11-15, Major 2).
4. Refining ADR-0090 decision 8: every repeat (a rerun, a batch rerun, a reset, a pending reset), a schedule's
   trigger-now and a private rerun of row 13 answer `field` `/` and `rule` `other`, whatever the caller may read
   (D043-121, D043-144, D043-148). The graded-disclosure check that would show the field to a caller who may read the
   source input is not built (G043-040).
5. Refining ADR-0090 decision 10: in `report` mode a start that an open holder answers creates no run. Its audit MUST be
   written with the holder's answer: with the receipt that answers its retries for a Signal-with-Start or an
   Update-with-Start, so once; in a statement of its own for a plain start answered by its holder, which keeps no
   receipt, so once per answer, as each answer is counted.

## Considered options

1. **The narrow `rt` exception, confined to one file by a rule** — chosen. Two merged designs keep working, the gate of
   design 043 still names no runtime, and the rule fails on a second use anywhere else in the crate.
2. **Move design 042's parse behind a port of an adapter now and restore ADR-0090's rule** — the clean way back
   (G043-052), but it changes a merged design that another session owns, in the middle of a release; left as the
   follow-up of design 042.
3. **Keep the rule that refuses any normal `tokio`, and fail the build** — breaks design 042 on `develop`.
4. **A composition-root test in each app instead of the wiring rule** — needs a database and the whole composition for
   each service; the rule is cheap, runs everywhere, and fails on the next service that forgets the gate.

## Consequences

- `sagat-application` holds a runtime dependency again, for one file; a second blocking task in the crate needs this
  ADR revisited (or the port of option 2).
- The wiring rule reads builder chains by text: a creator built through a helper that adds the gate elsewhere must
  keep `.with_run_inputs` in the chain that calls `new`, which every composition root does today.
- A schema with a name longer than 1 KiB, publishable before 0.3.0, is refused by 0.3.0 publication in `enforce` and
  refuses its starts, like every schema outside the subset (OD-2026-11-14-14); the inventory lists it.
- A caller who may read the source input of a repeat learns less than ADR-0090 allowed (`/`), never more.
- A plain `report`-mode start that an open holder answers writes one audit row per answer.
- Rolled back with the code: nothing is stored by this decision.

## Evidence

- Design: `docs/designs/043-run-input-validation.md`; registers: `specs/043-run-input-validation/research.md`
  (D043-101, D043-102, D043-105, D043-120, D043-121, D043-144, D043-148, D043-150, D043-154, D043-157 to D043-163,
  G043-040, G043-051, G043-052).
- Review reports: `docs/designs/043-run-input-validation.reviews/2026-11-15/` (architecture 1, 3, 7; security 1, 2;
  correctness 2, 4, 5, 6) and their resolution map.
- Tests seen failing first: the three fixtures of `TheApplicationHoldsNoRuntime` and the wiring fixture; the heaviest
  schema weighed 23.1 MB of a 16.8 MB share before the lazy DFA caches were lowered (7.4 MB after, compiled in 16 ms in
  the test build); the holder audit counted 0 before the fix.
