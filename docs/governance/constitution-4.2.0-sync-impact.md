# Constitution 4.2.0 — Sync Impact Report

This record accompanies the pull request that stops a merge into `develop` from starting the CI.
**Status: ADOPTED on 2026-11-05** by merging the pull request that carries it, on the owner's instruction (row
OD-2026-11-05-1 of [`docs/owner-decisions.md`](../owner-decisions.md)): "Desativar o ci para merges na develop. Deixar o
ci somente para merges de release".

## Scope and why MINOR

One clause of one bullet of Development Workflow changes: **CI gates**. Before, a pull request, and a push to `develop`,
MUST pass the unit tests of the packages it changes and the integration suites that touch the affected code. Now only a
pull request MUST, and a merge into `develop` starts no run. A release is unchanged. No principle changes and the
specification process is unchanged, so the amendment is not MAJOR; the guidance changes materially, so it is not PATCH.

The obligation is **relaxed for the push that merges a pull request**: the pull request has already passed the tests of
its change, and the push would run them again on the same code. The consequence is that two pull requests that are each
green but break each other are found when the next pull request that touches the same code runs, or when a release is
closed, and not when the second one is merged. The cost is the owner's to weigh; it is recorded here.

## Complete clause impact

| Unit of 4.1.0 | Disposition | Result |
|---|---|---|
| Core Principles I to VIII | KEEP | Byte-identical; hashes in the table below |
| Delivery Constraints | KEEP | Unchanged |
| Development Workflow: CI gates | REWRITE | The gates of a pull request lose "and a push to `develop`"; one sentence says a merge into `develop` starts no run; the gates of a release are unchanged |
| Every other Development Workflow bullet | KEEP | Unchanged |
| Governance | KEEP | Unchanged |
| Footer | REWRITE | `Version: 4.2.0`, `Last Amended: 2026-11-05`, the link to this report and the adoption sentence |

## Principle hashes

Previous-source: e90d66d2476179a0749e649a13d2bade8e3fd3d1

The line above is the git blob of `docs/constitution.md` at 4.1.0. A principle block is hashed as in the 4.0.0 report.

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

`.github/workflows/ci.yml` (the `push` trigger and the comment of the `changes` job), `scripts/tests/test_ci_workflow.py`
(fails when a push to `develop` starts the CI or a pull request stops starting it), the docstring of
`scripts/select-test-targets.py`, `docs/test-strategy.md` and `docs/owner-decisions.md`.

## Risks and rollback

- `develop` is no longer tested after each merge. A pull request is tested against the tip of `develop` as it was when
  its run started, so a merge that landed meanwhile is not in that run.
- A failure that only the combination of two merged changes shows is found by the next run that covers the code, or at
  the release.
- Rollback is a forward amendment that restores the clause, and a revert of the `push` trigger.
