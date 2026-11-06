# Constitution 4.3.0 — Sync Impact Report

This record accompanies the pull request that narrows what the CI of a pull request runs.
**Status: ADOPTED on 2026-11-06** by merging the pull request that carries it, on the owner's instruction (row
OD-2026-11-06-1 of [`docs/owner-decisions.md`](../owner-decisions.md)): "O que deve ter é execucao de testes do que foi
alterado ou modificado e seus dependentes, e integração também só do que foi alterado".

## Scope and why MINOR

One clause of one bullet of Development Workflow changes: **CI gates**. Before, a pull request MUST pass the unit tests of
the packages it changes and the integration suites that touch the affected code, that is, every suite whose sources name a
changed package or a package that depends on it. Now a pull request MUST pass the unit tests of the packages it changes
**and of the packages that depend on them**, and the integration suites **that it changes**, and only those. A merge into
`develop` still starts no run and a release still runs the whole stack. No principle changes and the specification process
is unchanged, so the amendment is not MAJOR; the guidance changes materially, so it is not PATCH.

The obligation is **tightened for unit tests and relaxed for integration**. A break in a core package now shows in the
unit tests of what is built on it, which is cheap. A break that only an integration suite would show, in a suite nobody
touched, is no longer found by the pull request: it is found by the next change to that suite or when a release is closed.
The cost is the owner's to weigh and is recorded here; it is the trade the owner asked for on 2026-11-04 for integration
suites ("só rodem quando modificados ou adicionados").

## Complete clause impact

| Unit of 4.2.0 | Disposition | Result |
|---|---|---|
| Core Principles I to VIII | KEEP | Byte-identical; hashes in the table below |
| Delivery Constraints | KEEP | Unchanged |
| Development Workflow: CI gates | REWRITE | The unit tests of a pull request include the packages that depend on a changed one; the integration suites are those the pull request changes, and no longer those whose sources name an affected package |
| Every other Development Workflow bullet | KEEP | Unchanged |
| Governance | KEEP | Unchanged |
| Footer | REWRITE | `Version: 4.3.0`, `Last Amended: 2026-11-06` and the link to this report |

## Principle hashes

Previous-source: ee4fb407c9deba50d1dddd158bbd9c25a869679c

The line above is the git blob of `docs/constitution.md` at 4.2.0. A principle block is hashed as in the 4.0.0 report.

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

`scripts/select-test-targets.py` (the rule and its docstring; the function that read the sources of the suites is gone),
`scripts/tests/test_select_test_targets.py`, `scripts/tests/test_ci_workflow.py`, `docs/test-strategy.md`,
`docs/owner-decisions.md`, `tests/AGENTS.md`, `scripts/README.md` and `.claude/skills/verify/SKILL.md`. The comment of the
`changes` job in `.github/workflows/ci.yml` still describes the old rule and is **not** updated here: a change of the
workflow, even of a comment, runs the whole stack (a release rule), which is the cost this amendment removes. It is
corrected by the next change that has to touch the workflow.

## Risks and rollback

- A change to a core package no longer runs the integration suites that use it. The unit tests of the package and of its
  dependents run; whoever changes a package runs the suites they affect before the pull request (`tests/AGENTS.md`).
- Two pull requests that are each green but break each other through an untouched suite are found at the release.
- Rollback is a forward amendment that restores the clause, and a restore of rule (b) in the selector.
