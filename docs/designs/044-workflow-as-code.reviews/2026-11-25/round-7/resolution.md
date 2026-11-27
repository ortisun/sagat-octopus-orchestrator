# Resolution of round 7 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed commit of round 7: e0a23b09f7ddbe225e80110c2cf25f3da960b730

How the Majors of round 7 were answered, by the author under the owner's delegation (OD-2026-11-20-6,
OD-2026-11-24-4). The decisions are Decision 044-175 to Decision 044-188. Every Nit was applied.

Counts before fixes: design re-read Blocker 0, Major 6, Nit 4; implementation readiness re-read 0, 1, 8; security 0, 4,
8; architecture 0, 4, 7; correctness 0, 6, 11. Total Blocker 0, Major 21, Nit 38. Round 6 had Blocker 0, Major 24,
Nit 35; of round 6's Majors all are closed except two partly closed, answered below.

| Findings | Answer |
|---|---|
| design 1 (can-act count misses open-scope members) | The overlay's `scope` against the run's sagas in forward or aborting phase (Decision 044-175) |
| design 2 (no mechanism for the journal-derived order) | `issued_seq` on the overlay row; a code arm of `plan_entries` (Decision 044-176) |
| design 3 (nested scopes) | Allowed, the parent recorded; the parent's abort aborts open children; closing a parent with an open child refused (Decision 044-176) |
| design 4; correctness 3 (the retained-bytes window) | No window and no retry for `RETAINED_BYTES_LIMIT` (Decision 044-177) |
| design 5; security 1; correctness 6 (copied initiators) | Sequence 0; children of any parent; every approval of a run with rows (Decision 044-178) |
| design 6; security 3 (transport limits) | Every request within the unchanged 320 KiB; responses below 4 MiB (Decision 044-180) |
| readiness 1 (golden vectors) | A checked-in Node generator, independent of the Rust code (Decision 044-182) |
| architecture A (definition loaders) | `get_version`, every cache caller counted and classified (Decision 044-183) |
| architecture B (rules that run only at release) | Expiry parity and journal-insert rule as tests of `apps/execution-engine` (Decision 044-183) |
| architecture C (`platform_requirement_hold` defined twice) | Design 045's definition, compared at migration (Decision 044-184) |
| architecture D (closed REST enumerations) | No new values: the workflow-task step is not a `RuntimeNode`, reset points use `STEP_COMPLETED` (Decision 044-185) |
| security 2 (literal flow-control keys) | Business literals; the keyed digest in the spec and journal (Decision 044-179) |
| security 4 (overlay cache unbounded) | 256 MiB per process, a quarter per tenant (Decision 044-181) |
| correctness 1 (reset after a caught abort) | `compensation.planned` and outcomes `COPY` once settled; saga rows rebuilt (Decision 044-186) |
| correctness 2 (the history window) | One window, its own bound, ended in the crossing completion or by a pass; the claim raises the review (Decision 044-177) |
| correctness 4 (handoff lock order and refusals) | The continuation pass performs the handoff; refusals stated (Decision 044-187) |
| correctness 5 (abort with a member in flight) | The abort journals only `saga.aborting`; feature 003 plans after members settle (Decision 044-175) |
