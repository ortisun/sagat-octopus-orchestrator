# Constitution 4.1.0 — Sync Impact Report

This record accompanies the pull request that scopes the CI to the change and keeps the whole stack for a release.
**Status: ADOPTED on 2026-11-04** by merging the pull request that carries it, on the owner's instruction (row
OD-2026-11-04-2 of [`docs/owner-decisions.md`](../owner-decisions.md)): "Quero que somente os testes unitários das
alterações sejam executados e também os testes de integração que tocam nos arquivos afetados pelas edições. O que quero
que rode completo é quando eu fechar uma release [...] Deixe isso como regra."

## Scope and why MINOR

One bullet of Development Workflow changes: **CI gates**. Before, formatting, `clippy`, the workspace tests, the
architecture rules, the design-doc check, the contract checks, the compose smoke test and the advisory audit MUST pass
before every merge. Now a pull request, and a push to `develop`, MUST pass the unit tests of the packages it changes and
the integration suites that touch the affected code, and a release MUST pass the whole stack. No principle changes and
the specification process is unchanged, so the amendment is not MAJOR; the guidance changes materially, so it is not
PATCH.

The obligation is **relaxed for a pull request**: lints, the architecture, design-doc and contract checks and the
upgrade, compose and scale verifications no longer gate a merge to `develop`; they gate a release. The consequence is that
a violation of those checks reaches `develop` and is found when a release is closed. The author runs them locally before
a pull request (`AGENTS.md`, Commands; the `verify` skill).

## Complete clause impact

| Unit of 4.0.0 | Disposition | Result |
|---|---|---|
| Core Principles I to VIII | KEEP | Byte-identical; hashes in the table below |
| Delivery Constraints | KEEP | Unchanged |
| Development Workflow: CI gates | REWRITE | Split in the gates of a pull request or of a push to `develop` (unit tests of the changed packages, integration suites that touch the affected code) and the gates of a release (the whole stack); a release is defined |
| Every other Development Workflow bullet | KEEP | Unchanged |
| Governance | KEEP | Unchanged |
| Footer | REWRITE | `Version: 4.1.0`, `Last Amended: 2026-11-04`, the link to this report and the adoption sentence |

## Principle hashes

Previous-source: 9518d35052bbe22b12852fe8fb56fc4f4bb34bf1

The line above is the git blob of `docs/constitution.md` at 4.0.0. A principle block is hashed as in the 4.0.0 report.

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

`.github/workflows/ci.yml` (the `changes` job and the gates of every other job), `scripts/select-test-targets.py` and its
tests, `scripts/tests/test_ci_workflow.py` (fails when a lint, a verification job or an integration suite loses its gate),
`AGENTS.md`, `docs/test-strategy.md`, `tests/AGENTS.md`, the `verify` skill, `scripts/README.md` and `docs/owner-decisions.md`.

## Risks and rollback

- Code changed without a change to its tests runs only the unit tests of its package and the suites that name an affected
  package. A suite that exercises the code through HTTP without naming its package is missed until the release.
- The selection of the integration suites is static (which packages a suite's own sources name); a coverage map would be
  more exact and is not built.
- Rollback is a forward amendment that restores the previous bullet, and a revert of the workflow.
