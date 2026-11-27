# Resolution of round 11 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed commit of round 11: 9bdb707d84f3fef3c190979cde128256f2b8159a

How the Majors of round 11 were answered, by the author under the owner's delegation (OD-2026-11-20-6,
OD-2026-11-24-4). The decisions are Decision 044-223 to Decision 044-229. Every Nit was applied.

Counts before fixes: design re-read Blocker 0, Major 3, Nit 6; implementation readiness re-read 0, 1, 7; security 0, 4,
8; architecture 0, 3, 11; correctness 0, 4, 8. Total Blocker 0, Major 15, Nit 40. Round 10 had Blocker 0, Major 17,
Nit 44.

| Findings | Answer |
|---|---|
| design 1 (SDK-issued commands unbounded) | 147 SDK commands, 64 open updates, one consumption, scope closes counted (Decision 044-223) |
| design 2 (re-emitted held blob) | Named by descriptor, no upload, no double charge (Decision 044-224) |
| design 3; architecture 3 (chain lock order) | The member-chain lock every run transaction takes first; no inversion (Decision 044-226) |
| readiness 1 (unserved alert mechanism) | The in-app kind `WORKFLOW_TASK_UNSERVED` with its contracts, migration and runbook (Decision 044-229) |
| architecture 1 (stops the function caused) | A closed `CancelOrigin`; every `stop_run(` call site counted (Decision 044-227) |
| architecture 2 (graph node named `c-1`) | `steps.code_step` (Decision 044-227) |
| correctness 1 (a run at its 4 MiB cannot end) | Terminal output and continue-as-new values leave the run (Decision 044-225) |
| correctness 2 (tenant blob cap) | Per-run bound apart from the tenant's count; at most 16 per completion (Decision 044-224) |
| correctness 3 (history blob and the canonical descriptor) | Only lease-written or held blobs; history values uploaded again (Decision 044-224) |
| correctness 4 (poison replay check) | A 300 s lease, three expiries end the item (Decision 044-229) |
| security 1 (chain-addressed idempotency) | Keys and signals keep the address's scope (Decision 044-228) |
| security 2 (semaphore capacity) | 64 MiB, a deadline, `UNAVAILABLE` not counted (Decision 044-224) |
| security 3 (gated input as a blob) | Always inline (Decision 044-224) |
| security 4 (declassification on a move) | The accepting version's carrier (Decision 044-228) |
