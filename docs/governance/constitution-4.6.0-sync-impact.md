# Constitution 4.6.0 — Sync Impact Report

This record accompanies the pull request that lets a change of a contract file select the contract suites and the
packages that read it. **Status: PROPOSED**; it is adopted by merging the pull request that carries it, on the owner's
delegation of the 0.3.0 decisions to the coordinating session (OD-2026-11-20-6) and the delegate's decision
OD-2026-11-20-7 of [`docs/owner-decisions.md`](../owner-decisions.md), which answers finding m7 of the architecture
re-review of design 022 (`specs/022-flow-control-step-policies/reviews/2026-11-20/keys-rereview-architecture.md`). The
owner may overrule it by a later amendment.

## Scope and why MINOR

One clause of one bullet of Development Workflow changes: **CI gates**. Before, a pull request MUST pass the unit tests
of the packages it changes and of their dependents, the integration suites it changes, the budget suite for the hot
path and the step-log suites for the step-log code, and only those; a contract file under `specs/<feature>/contracts/`
was a shared input that selected nothing (OD-2026-11-06-2). Now it MUST also pass, when it changes a contract file, the
contract suites (`tests/contract/`) and the unit tests of the packages that read that file and of the packages that
depend on those. No principle changes and the specification process is unchanged, so the amendment is not MAJOR; it adds
an obligation, so it is not PATCH.

Why: the contract suites exist to hold the code to the contracts, and they read the contract to do it. A pull request
that changed only `openapi.yaml`, `worker.proto`, `events.asyncapi.yaml` or `workflow.schema.json` ran none of them, and
not even the build of `sagat-api-grpc`, which compiles `worker.proto`: a contract regression could merge untested and be
found only at the release. Integration, fault, e2e and performance suites that also read a contract are not selected by
it; they cost the delivery time OD-2026-11-04-1 protects, and the release runs them.

## Complete clause impact

| Unit of 4.5.0 | Disposition | Result |
|---|---|---|
| Core Principles I to VIII | KEEP | Byte-identical; hashes in the table below |
| Delivery Constraints | KEEP | Unchanged |
| Development Workflow: CI gates | REWRITE | A pull request that changes a contract file also runs the contract suites and the unit tests of the packages that read it |
| Every other Development Workflow bullet | KEEP | Unchanged |
| Governance | KEEP | Unchanged |
| Footer | REWRITE | `Version: 4.6.0`, `Last Amended: 2026-11-20`, the link to this report and the adoption sentence |

## Principle hashes

Previous-source: 9bd79a31e2a61e7900d7a15f07c6e9a4035d34e6

The line above is the git blob of `docs/constitution.md` at 4.5.0. A principle block is hashed as in the 4.0.0 report.

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

## Dependents

The selector rule (`scripts/select-test-targets.py`, class `Readers`) and its tests (`scripts/tests/test_select_test_targets.py`)
change in the same pull request, with `docs/test-strategy.md`, `tests/AGENTS.md`, `scripts/README.md` and
`docs/owner-decisions.md`. Who reads a contract is derived from the Rust sources at every run (a string literal that is a
path to the file, or to its directory joined with its name), not kept in a table; a test over the real repository fails
when a contract suite or a package names a path into a contracts directory that the rule does not see it read, so a new
reader that builds the path another way is caught. `AGENTS.md` already says that a pull request runs "the integration
suites that touch" what it changes, which covers the clause. `.github/workflows/ci.yml` is not touched: it runs the
targets and packages the selector names.

## Risks and rollback

- A pull request that changes `specs/001-platform-foundation/contracts/openapi.yaml` runs fifteen contract suites today
  (the REST conformance suites start PostgreSQL), and one that changes `workflow.schema.json` the unit tests of
  `sagat-infra-ir` and its dependents: minutes, not seconds. That is the intent.
- The rule is static: a reader that builds a contract path from parts the selector does not see is missed until the
  real-repository test flags it. SDKs, the Studio and the lake runtime that read contracts (sync scripts, generators)
  are not covered by this clause; their own files still select them, and the release runs them.
- Rollback is a forward amendment that removes the clause, and the removal of the rule in the selector.
