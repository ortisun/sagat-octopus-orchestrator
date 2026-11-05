# Architecture review of the built change (PR 1 and PR 2)

Doc: docs/designs/constitution-4.0.0-egm.md
Reviewed-commit: 072481c8673916b5a5b870a6e9f43b37d4a5b4f6
Reviewer: architecture
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The reviewer read files only and could not compare against the 3.0.0 text (the blob was not on disk for it) or recompute hashes; both are checked by rule R8 of `scripts/check-design-docs.py`, which passes.

## Major

1. **The test that guards the "not yet converted" wording does not exist, and one dependent lacks the wording.** `security-reviewer.md` line 10 had no alternative wording; the design promised a test in section 2, the Risks table and TEGM-26 and called the files "six" while naming seven.
   *Resolution:* fixed in cf528bee. `TransitionalWording` in `test_check_agent_config.py` checks the seven files (it failed first on `security-reviewer.md`); the reviewer now carries the wording; the design says seven.
2. **The checklist gate that the deleted implement skill enforced has no carrier.** Feature 036's tasks and checklist and feature 020's checklist cite it; the design recorded "None".
   *Resolution:* fixed in cf528bee. The transitional clause of the constitution keeps the gate (unticked `checklists/*.md` items block implementation until the owner accepts them); features 020 and 036 no longer cite the deleted skill; the Sync Impact Report, ADR-0077 and the design say review is the only control.

## Nit

1. `docs/designs/README.md` said the constitution lands later. *Resolved:* sentence removed.
2. `architecture-principles.md` overstated R8. *Resolved:* reworded to "principle hashes agree with the report and a changed constitution raises its version".
3. TEGM-26 said `check()` does not change. *Resolved:* "except the settings guard of TEGM-4".
4. Leftover wording ("the diff and the spec"; `parallel-agents` line 13). *Not changed:* wording only.
5. The narrowed "approved dependency" is unchecked. *Not changed:* the design already states that R5 checks ticked gated tasks only (section 5).
6. The Sync Impact Report omitted dependents. *Resolved:* list completed.

Counts: Blocker 0, Major 2, Nit 6
