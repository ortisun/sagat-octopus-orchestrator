# Constitution 4.5.0 — Sync Impact Report

This record accompanies the pull request that lets a change of the step-log code select the step-log suites. **Status:
PROPOSED**; it is adopted by merging the pull request that carries it, on the owner's decision OD-2026-11-15-10 of
[`docs/owner-decisions.md`](../owner-decisions.md): "Emendar o gate de CI" ("amend the CI gate"), which answered O041-022
of design 041 (step logs). The constitution asks for a pull request that changes this file, the version bump and this
report (Governance, Amendments); it does not ask the owner to sign the exact text, and the owner may overrule it by a
later amendment.

## Scope and why MINOR

One clause of one bullet of Development Workflow changes: **CI gates**. Before, a pull request MUST pass the integration
suites it changes and, when it changes the code that runs a lease or a completion, the budget suite, and only those. Now
it MUST also pass, when it changes the code that captures, sends, stores or serves step logs (the server's step-log
modules, the worker-protocol contract, or an SDK's capture code), the step-log suites: `integration_step_logs` and,
with it, the budget suite `integration_claim_transition_efficiency` for the server's step-log code and the SDK
conformance suite `sdk_conformance` for the worker-protocol contract and an SDK's capture code. No principle changes and the
specification process is unchanged, so the amendment is not MAJOR; it adds an obligation, so it is not PATCH.

Why: the step-log suites guard behaviour that lives in code they do not contain. A fix in the ingest's application module,
in the admission class of `crates/api-grpc`, in the read routes, in the worker protocol or in an SDK's capture code can
break redaction, the tenant boundary or the completion's wait without touching a test, and under 4.4.0 such a pull
request runs none of them (design 041, O041-022; implementation-readiness re-audit entry 23).

## Complete clause impact

| Unit of 4.4.0 | Disposition | Result |
|---|---|---|
| Core Principles I to VIII | KEEP | Byte-identical; hashes in the table below |
| Delivery Constraints | KEEP | Unchanged |
| Development Workflow: CI gates | REWRITE | A pull request that changes step-log code also runs the step-log suites |
| Every other Development Workflow bullet | KEEP | Unchanged |
| Governance | KEEP | Unchanged |
| Footer | REWRITE | `Version: 4.5.0`, `Last Amended: 2026-11-15`, the link to this report and the adoption sentence |

## Principle hashes

Previous-source: 042e45eb591e6660604006c956730c8092b2ec07

The line above is the git blob of `docs/constitution.md` at 4.4.0. A principle block is hashed as in the 4.0.0 report.

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

The suites the clause names do not exist yet: `integration_step_logs` is created by T041-6 of design 041 and the SDK
scenarios `step-logs-*` by T041-14. The selector rule that applies the clause (`scripts/select-test-targets.py`, its tests
in `scripts/tests/test_select_test_targets.py`), `docs/test-strategy.md`, `tests/AGENTS.md` and `scripts/README.md` are
changed by T041-32, which lands with or after T041-6; until then the clause has nothing to select, and that lag is
stated here rather than hidden. `AGENTS.md` already says that a pull request runs "the integration suites that touch"
what it changes, which covers the clause. The comment of the `changes` job in `.github/workflows/ci.yml` is not touched,
for the reason the 4.3.0 report gives.

## Risks and rollback

- A pull request that changes step-log code or an SDK's capture code builds and runs the step-log suites, which include
  real PostgreSQL and, for an SDK, its conformance scenarios: minutes, not seconds.
- Every later change of `worker.proto`, by any feature (020, 021, 025 and 026 each plan one), runs the six-language
  conformance suite and `integration_step_logs`: today such a change, under `specs/.*/contracts/`, selects nothing on a
  pull request. That is up to an hour per such pull request, and the intent: the protocol is what the SDKs share.
- The paths are the ones design 041 names; step-log behaviour reached from another path is found at the release.
- Rollback is a forward amendment that removes the clause, and the removal of the rule in the selector.
