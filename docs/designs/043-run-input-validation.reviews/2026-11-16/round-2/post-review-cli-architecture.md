# Check of the CLI fix after the reviewed commit of design 043: architecture (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: f48d5729eb534dc25c6cb04eeeabb0dc80202bad
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author; every finding and severity is kept. The final gate run at 4b6eb1a6 failed one unit test of
`sagat-cli` (`VALIDATOR_BUSY` had no help); the fix adds one catalog entry in `apps/cli/src/diagnostics.rs` and its section
in `docs/quickstarts/errors.md`. This report judges that diff alone; it is not one of the four reports of the approval.

No architecture problem: app layer and docs only, no new decision (the design already names the code), additive. The fix
can land after 4b6eb1a6 without changing the approval's reviewed commit; the gate rerun must show the CLI tests pass.

## Nit

1. The help hard-codes "5 s", the value of `RUN_INPUT_PERMIT_WAIT`, with no test tying them (as the design and the
   errors contract already do).

Counts: Blocker 0, Major 0, Nit 1
