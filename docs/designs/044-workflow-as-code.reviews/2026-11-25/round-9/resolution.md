# Resolution of round 9 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed commit of round 9: 842f013e1e19a2d4e09617fc0abf88296275b860

How the Majors of round 9 were answered, by the author under the owner's delegation (OD-2026-11-20-6,
OD-2026-11-24-4). The decisions are Decision 044-201 to Decision 044-212 and Gap 044-027. Every Nit was applied.

Counts before fixes: design re-read Blocker 0, Major 3, Nit 11; implementation readiness re-read 0, 4, 8; security 0, 4,
6; architecture 0, 3, 16; correctness 0, 2, 9. Total Blocker 0, Major 16, Nit 50. Round 8 had Blocker 0, Major 18,
Nit 43; the correctness review closed both of its round-8 Majors.

| Findings | Answer |
|---|---|
| design 1; correctness 1 (blob-carried values and the chain) | The descriptor, never the blob identifier, in the hashed body; the deferral holds its blob (Decision 044-201) |
| design 2 (more than 256 commands in one activation) | A deterministic `CommandBatchLimitExceeded`; a scenario (Decision 044-202) |
| design 3 (closing window too small) | Three per step that can still act, the bound capped at 58,368 (Decision 044-203) |
| readiness 1 (test environments against the preconditions) | Code-workflow workers under UTC and a fixed seed; injected readers (Decision 044-204) |
| readiness 2 (`CommandDigester` undefined) | Two methods; the domain folds; a fold test (Decision 044-205) |
| readiness 3 (packaging of the new distributables) | `package-sdks.sh`, the packaging workflow, runbook and release notes in Tasks 044-10, 044-17 and 044-18 |
| readiness 4 (red tests that wait on a later task) | `#[ignore = "green with Task 044-NN"]` and a release gate (Decision 044-204) |
| architecture A (move items and feature 018's model) | Mapped to `SUCCEEDED`, `SKIPPED`, deferred and `FAILED` (Decision 044-207) |
| architecture B (workflow type uniqueness) | `code_workflow_types`; a barrier test (Decision 044-208) |
| architecture C (cache filled before commit) | Filled after commit; dropped on mismatch; a test (Decision 044-209) |
| correctness 2 (continue-as-new without a chain) | Refused unless the run heads an open chain (Decision 044-210) |
| security 1 (binder escapes in other runs) | Written into `run_initiators` by the claim (Decision 044-211) |
| security 2 (cache thrashing) | Cold reads of the addressed step only; a numbered byte ceiling (Decision 044-209) |
| security 3 (no immediate operator stop) | Cancellation without grace under `run:operate` (Decision 044-211) |
| security 4 (carriers against feature 016) | Kept outside the carriers, raw payloads `pii` (Decision 044-212) |
