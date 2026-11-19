# Resolution of the review run of 2026-11-15 (design 043)

Each finding of the four reports in this directory, and what answers it. FIXED: a code change with a test seen failing
first (commit on `feature/043-creators`). TEXT: the design, ADR-0092 or a contract now says what was built, at the place
named. CONTEST: the finding is not what the code does, with the evidence. OWNER: a product or scope question, put to
the owner in `specs/043-run-input-validation/research.md` with options and a recommendation; the design states what is
built meanwhile. Registers: D043-157 to D043-163, G043-053, G043-054, O043-005, O043-006.

## architecture.md

| # | Severity | Resolution |
|---|---|---|
| 1 | Blocker | FIXED and TEXT. ADR-0092 (Proposed) records the `rt` exception as a refinement of ADR-0090; `run_input_runtime_errors` now fails on `tokio` anywhere in `crates/application/src/**` but `collection_source.rs` and test modules, with negative fixtures (`test_tokio_is_named_only_by_the_blob_source_parse_and_the_tests`, red first; commit d33d9c61). FR-043-016, Technical Plan 3 ("Where the runtime lives"), the Constitution Check row II, Complexity Tracking and T043-4 corrected. |
| 2 | Major | FIXED and TEXT. The exemption is keyed by (file, function) (`LEGACY_FORWARDER`); `test_a_forwarder_named_continue_as_new_elsewhere_is_refused` red first (d33d9c61); FR-043-016 states it (D043-160). |
| 3 | Major | FIXED and TEXT. The seventh rule, `gate_wiring_errors`, fails on a composition root that builds a creator without `.with_run_inputs` (`test_a_composition_root_wires_the_gate_into_every_creator_it_builds`, red first; d33d9c61; ADR-0092 decision 2, D043-161). Technical Plan 3 describes `unwired()` ("A service without a gate"). |
| 4 | Major | TEXT. T043-14 carries `Owner-gate: OD-2026-11-15-3` and the re-pinned baselines; Technical Plan 9 says which operations describe their `422` (the six start operations) and that the schedule writes, trigger-now and publication rely on the shared `Error` schema and `errors.md` (D043-135, D043-149). The decision row now reads `Decided` (D043-163). |
| 5 | Major | FIXED. `failureCode` on the REST `Run` from `runs.stop_code`, the six SDK `Run` types, the Studio types and the conformance goldens (D043-162, closes G043-046); FR-043-015, Technical Plan 9, T043-14 and T043-15 say so. |
| 6 | Major | TEXT. The amendments block is gone; each item is in its section: the `oneOf` bound (pre-check step 4), the repeat policy (FR-043-003, Technical Plans 1 and 5), the two schedule reads (Technical Plan 2), row 14's time (FR-043-006, Technical Plan 5, now the next nominal time by D043-158), the backfill without a pause (FR-043-006, row 7), the two migrations (Technical Plan 5), the tenant share (Cache), `check_schema`, `recheck` and `Interrupted` (the sketch of Technical Plan 3), the metrics (Technical Plan 7). |
| 7 | Major | TEXT. ADR-0090 is accepted and is not edited in substance; ADR-0092 decision 3 records the name limit (a narrowing), the `oneOf` bound and the tenant share, decisions 4 and 5 the repeat field and the holder audit. ADR-0090's status line points to ADR-0092 and its Evidence gives the register range now. |
| 8 | Nit | TEXT. Date 2026-11-16; the Overview defines phases 1 to 3 under OD-2026-11-14-15. |
| 9 | Nit | TEXT. Register ranges in the Overview and ADR-0090's Evidence. |
| 10 | Nit | TEXT. FR-043-012 and Technical Plan 8: `CLASSIFICATION_TOO_COMPLEX` in both modes. |
| 11 | Nit | TEXT. Technical Plan 10 names the down migrations' refusals and that the binaries roll back without them. |
| 12 | Nit | FIXED. `describe` says "the run input could not be checked now; will retry" (`a_start_waiting_for_the_validator_is_not_described_as_storage`; d33d9c61). |
| 13 | Nit | Left as a Nit: `docs/engineering/architecture-principles.md` gains no rows in this delivery; the seven rules are listed in FR-043-016 and ADR-0090/ADR-0092 "Enforced by". |
| 14 | Nit | CONTEST, written in Technical Plan 3: the domain cannot hash (`sha2` is not in `DOMAIN_ALLOWED`), the codec is the only production constructor, and a wrong digest only mis-keys the cache within its own tenant, whose id is in the key. |
| 15 | Nit | TEXT. `specs/043-run-input-validation/contracts/errors.md` notes the two meanings of `INPUT_INVALID`, and Technical Plan 9. |
| 16 | Nit | FIXED. Quoted and qualified names and the up migrations (`test_a_qualified_or_quoted_table_name_and_a_migration_are_read_too`, red first; d33d9c61). |

## security.md

| # | Severity | Resolution |
|---|---|---|
| 1 | Major | TEXT. FR-043-003, the rows 4, 5, 8 and 13 of Technical Plan 1, Technical Plan 5 and ADR-0092 decision 4: repeats, trigger-now and the private rerun answer `/` and `other` to every caller (D043-121, D043-144, D043-148). Whether to build graded disclosure later is OWNER O043-006. |
| 2 | Major | FIXED. The lazy DFA caches are 16 KiB per direction, so the heaviest admitted schema weighs 7.4 MB of a 16.8 MB share and is kept; `the_heaviest_schema_the_precheck_admits_fits_a_tenant_share_and_is_kept` red first (23.1 MB), its compile time measured (16 ms, test build; 9a6296dc, D043-159). NFR-043-BOUND, the Cache paragraph and Appendix A say so. |
| 3 | Nit | TEXT. As security 1. |
| 4 | Nit | TEXT. Technical Plan 3 (the backstop paragraph) and a Risks row state it; G043-053 open. |
| 5 | Nit | FIXED (text and comment). The inventory runs with the control plane's database settings (`config.database`), not the owner's: Technical Plan 8, the runbook and `apps/control-plane/src/inventory.rs` corrected (D043-163). |
| 6 | Nit | TEXT. Technical Plan 8: it prints keywords and pointers, which are author text, in double quotes with escapes (D043-111). |
| 7 | Nit | TEXT. Technical Plan 3 ("Threads and permits"): one queue entry per waiting caller, bounded by the callers in flight and the 5 s wait; the sentence on a tenant's third permit no longer contradicts itself. |
| 8 | Nit | CONTEST with TEXT: by design the action check is the authority (D043-008, D043-125), and the seventh rule now fails on a composition root that builds a `ScheduleService` without the gate; Technical Plan 3 says so. |
| 9 | Nit | Left as a Nit: the sentinel covers rows 1 to 6, 8, 9 and 11 to 14, the schedule records, the journal's stops and the metrics text (G043-050); batch items, pending resets, the parent's failure event and the inventory output carry codes, not inputs, and are not swept in this delivery. |

## correctness.md

| # | Severity | Resolution |
|---|---|---|
| 1 | Major | FIXED before this map (de26320a, D043-156): the tick's audit is written with the compare-and-set move of its trigger; the two fault tests were red first (2 audits each). FR-043-007, Technical Plan 5 and T043-7 cite it. |
| 2 | Major | FIXED. The audit of a `report`-mode start an open holder answers is written with the holder's receipt, or with the answer of a plain start (`report_mode_audits_a_start_that_an_open_holder_answers`, red first with 0 audits; d3eb4aab, eeff9a1a; D043-157, ADR-0092 decision 5). FR-043-013 and Technical Plan 6 say so. |
| 3 | Major | FIXED. The write fills the next nominal time and resolves the declaration through `$ref` and `allOf` (`row_14_the_write_fills_the_next_nominal_time_and_resolves_the_declaration`, red first; eeff9a1a; D043-158). CONTEST in part: the write stays stricter in the one rule 018's contract states (the field must be declared), written in Technical Plan 5. |
| 4 | Major | FIXED, as security 2. |
| 5 | Major | TEXT as security 1; OWNER O043-006 for the graded check. |
| 6 | Major | As architecture 1. |
| 7 | Major | FIXED, as architecture 5. |
| 8 | Major | TEXT and OWNER. 016's contract (section 1.3, rule 6) holds an `inputSchema` carrier to the subset; a Risks row; Technical Plan 9 and T043-14. The 035 tuple regression is O043-005 (recommended: accept for 0.3.0). |
| 9 | Major | TEXT. D043-120 (Technical Plan 3, "A service without a gate"), D043-122 (FR-043-016), D043-126 and G043-041 (Technical Plan 2 and 5), D043-134 (FR-043-006, row 7), D043-142 (FR-043-006, Technical Plan 5), D043-144 (row 8), D043-146 (FR-043-006, Technical Plan 5). |
| 10 | Nit | TEXT. The sketch's `SchemaRefusal` has `Busy`, `Backstop` and `Interrupted`; Technical Plan 1 lists the three transient cases; the panic is G043-053. |
| 11 | Nit | TEXT. Technical Plan 3: a third or fourth permit only while no other tenant waits. |
| 12 | Nit | TEXT. CUJ 5: every attempt counted, each run or holder answer audited once. |
| 13 | Nit | TEXT. The Overview and ADR-0090's Evidence. |
| 14 | Nit | CONTEST in part, with TEXT: a dispatch waiting for the validator uses up none of the event's attempts (`is_waiting_for_room`), and only an event that waited a day ends `DEAD`; Technical Plan 1 says so, G043-054. |
| 15 | Nit | FIXED. Seen red by a hand mutation (a recheck that counts like a check: 2 counted), reverted (D043-163). |

## zoom_out_report.md

| # | Severity | Resolution |
|---|---|---|
| 1 | Major | As architecture 1. |
| 2 | Major | As architecture 6; NFR-043-BOUND gains the 1 KiB name limit. |
| 3 | Major | TEXT. The search's colouring, the off-worker step and the `Condvar`, the regex engine settings and the map order moved to Appendix A, which says it is informative and held by the tests of T043-2 and T043-3. CONTEST in part: the pattern allowlist stays in the subset table, because it is the contract an author writes against (`INPUT_SCHEMA_PATTERN_UNSUPPORTED`). |
| 4 | Nit | TEXT. Register ranges; Open Question 2 gives the measured development-machine number and leaves CI and production open. |
| 5 | Nit | TEXT. Technical Plan 1 says the line numbers drift and the symbol beside each is what to search. |
| 6 | Nit | TEXT. T043-3 says what is built in `application` and what in the adapter. |
| 7 | Nit | CONTEST: privacy is per module, while the rule covers every crate and test path that depends on `sagat-application` with `test-support`, and a `Default` or `Deserialize` written in the gate's own module, which privacy cannot stop. |
| 8 | Nit | CONTEST: a shared helper would need a crate both adapters depend on, a new edge for eight lines; Technical Plan 3 keeps the twin. |
| 9 | Nit | TEXT. Complexity Tracking records the plain semaphore with a timed acquire and why it lost. |
| 10 | Nit | Left as a Nit: removing the tests-only legacy continue-as-new belongs to the continuation chains, not to this design; it stays a follow-up. |
| 11 | Nit | TEXT. Alternatives Considered records "the subset's pre-check, then `jsonschema`". |
