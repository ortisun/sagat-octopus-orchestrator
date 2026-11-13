# Constitution 4.4.0 — Sync Impact Report

This record accompanies the pull request that lets the change of the engine's hot path select the suite that budgets it.
**Status: PROPOSED**; it is adopted by merging the pull request that carries it, on the owner's decision OD-2026-11-12-4 of
[`docs/owner-decisions.md`](../owner-decisions.md): "Seguir as recomendações", which accepted, among three
recommendations, that the budget target runs on a pull request that changes the hot-path files.

## Scope and why MINOR

One clause of one bullet of Development Workflow changes: **CI gates**. Before, a pull request MUST pass the integration
suites that it changes, and only those. Now it MUST also pass, when it changes the code that runs a lease or a completion,
the suite that budgets what they cost the database (`integration_claim_transition_efficiency`, design 038). No principle
changes and the specification process is unchanged, so the amendment is not MAJOR; it adds an obligation, so it is not
PATCH.

Why one suite is selected by code it does not contain: the transitions scenario of feature 010 fell from 1,513 to 368
transitions per second between 2026-10-11 and 2026-11-11 (design 038, `specs/038-claim-transition-efficiency/research.md`).
Each change that did it was reviewed and green; none of them changed a suite that could have counted its statements. A
budget only protects the hot path if the change that breaks it runs it.

## Complete clause impact

| Unit of 4.3.0 | Disposition | Result |
|---|---|---|
| Core Principles I to VIII | KEEP | Byte-identical; hashes in the table below |
| Delivery Constraints | KEEP | Unchanged |
| Development Workflow: CI gates | REWRITE | A pull request that changes the code running a lease or a completion also runs the budget suite |
| Every other Development Workflow bullet | KEEP | Unchanged |
| Governance | KEEP | Unchanged |
| Footer | REWRITE | `Version: 4.4.0`, `Last Amended: 2026-11-12` and the link to this report |

## Principle hashes

Previous-source: a424aa5964bc528374a4ba42a3b97f9eaca51fa4

The line above is the git blob of `docs/constitution.md` at 4.3.0. A principle block is hashed as in the 4.0.0 report.

| Principle | Disposition | sha256 (previous) | sha256 (this version) |
|---|---|---|---|
| I | KEEP | 32d2ec982a4ae0995a472628184c4762ea740f9081886bb4c78fa461237a5a21 | 32d2ec982a4ae0995a472628184c4762ea740f9081886bb4c78fa461237a5a21 |
| II | KEEP | 80fbee5c64129b11cc2cc1d978c7a4b91f9af39cfdea73510c3bcb345aad8ec2 | 80fbee5c64129b11cc2cc1d978c7a4b91f9af39cfdea73510c3bcb345aad8ec2 |
| III | KEEP | 69d38cce2aa95cfa7f3e70b2be2e93260ff8f224afa7e8f779e5558b4f9827d9 | 69d38cce2aa95cfa7f3e70b2be2e93260ff8f224afa7e8f779e5558b4f9827d9 |
| IV | KEEP | ff10043d76dfd25ac3b2db1ea27b943804c52d4d46c83838e6908cdaccc32f7b | ff10043d76dfd25ac3b2db1ea27b943804c52d4d46c83838e6908cdaccc32f7b |
| V | KEEP | b816cb76163ef1d8cc8579bb3e326f59c774b077131029cfd64fa45d470f38f9 | b816cb76163ef1d8cc8579bb3e326f59c774b077131029cfd64fa45d470f38f9 |
| VI | KEEP | 3fbc81dd01da920dafb2681958b3a7864f90b6395d3aecb2254555436c6aa8bc | 3fbc81dd01da920dafb2681958b3a7864f90b6395d3aecb2254555436c6aa8bc |
| VII | KEEP | 708ea13f72890e3ff5bf0472c30fdb2a5cab3d5e616e40e88197c880eeba2760 | 708ea13f72890e3ff5bf0472c30fdb2a5cab3d5e616e40e88197c880eeba2760 |
| VIII | KEEP | df43387c86a14df13ffde3c5033971086a075c0b7e18fa538322c1f52b3d1a37 | df43387c86a14df13ffde3c5033971086a075c0b7e18fa538322c1f52b3d1a37 |

## Dependents updated in the same change

`scripts/select-test-targets.py` (a harness rule: `crates/infra-postgres/`, and the worker, execution and run modules of
`crates/application`, select `integration_claim_transition_efficiency`), `scripts/tests/test_select_test_targets.py`,
`tests/Cargo.toml` and `tests/integration/claim-transition-efficiency/` (the budget tests leave
`integration_durable_execution`, so that what runs on such a pull request is the budget and not a five-minute suite),
`docs/test-strategy.md`, `tests/AGENTS.md`, `scripts/README.md` and `docs/designs/038-claim-transition-efficiency.md`.
The comment of the `changes` job in `.github/workflows/ci.yml` is not touched, for the reason the 4.3.0 report gives.

## Risks and rollback

- A pull request that changes `crates/infra-postgres/` now builds and runs one more test binary: about ten tests, a few
  seconds of execution, plus its build.
- The budget is an equality on counts. A pull request that makes the hot path cheaper also fails it until it lowers the
  numbers, which is the intent: the budget moves in the change that earns it.
- The files named are the ones that ran the 2026-11 regression; a hot-path change elsewhere (a trigger in a migration of
  another crate, a domain change that adds a load) is not selected and is found at the release.
- Rollback is a forward amendment that removes the clause, and the removal of the rule in the selector.
