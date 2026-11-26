# Resolution of round 6 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed commit of round 6: b5c23b83c9a63c8ed42f15b4cdf881ecb683dd4e

How the Majors of round 6 were answered, by the author under the owner's delegation (OD-2026-11-20-6,
OD-2026-11-24-4). The decisions are Decision 044-159 to Decision 044-174 and Gap 044-025. Every Nit was applied.

Counts before fixes: design re-read Blocker 0, Major 5, Nit 8; implementation readiness re-read 0, 2, 7; security 0, 3,
4; architecture 0, 7, 12; correctness 0, 7, 4. Total Blocker 0, Major 24, Nit 35. Round 5 had Blocker 0, Major 12,
Nit 48; the round-5 Majors are all closed, and the new ones come from a fresh reading of the whole text.

| Findings | Answer |
|---|---|
| design 1 (code scopes without a step row) | A step row `s-<n>` waiting on the saga, never offered to a worker (Decision 044-165) |
| design 2 (the function never gets its input) | `S(0) = 0`; `run.started` visible and waking with the input and carried state (Decision 044-159) |
| design 3; correctness F (reset with an open update) | Such a point is ineligible; transitions rebuilt; `reapply: UPDATES` refused (Decision 044-163) |
| design 4; correctness G (bytes over the run's or the tenant's bound) | `RETAINED_BYTES_LIMIT` review with a closing window; tenant quota counted; no `stop_for_storage_quota` (Decision 044-166) |
| design 5 (hand-kept can-act counter) | Read by one statement over a partial index; 63 statements (Decision 044-167) |
| readiness 1; architecture 5 (call-site check counts generic names) | Distinctive names only, loaders held for the generic ones, a shrinking count fails (Decision 044-171) |
| readiness 2 (`timeoutLevel` undefined) | Only `"none"` until design 045 lands (Decision 044-172) |
| security 1; architecture 6; correctness E in part (initiators across runs) | `run_initiators`, copied by handoff, child start and reset; resolvers, resetters, publishers (Decision 044-164) |
| security 2 (blobs bypass the placeholder and redaction) | Never delivered to workflow, query or replay-check tokens; export by reference only (Decision 044-168) |
| security 3 (history streams and update waits uncapped per tenant) | Stream caps and a byte budget; RPC classes; 64 waits per tenant (Decision 044-168) |
| correctness A (history flags lease-only) | Journaled in `workflow_task.started` (Decision 044-160) |
| correctness B (continue-as-new after a cancellation request) | Ends `CANCELLED`, no handoff; latches mapped (Decision 044-161) |
| correctness C (move of a stopping run) | Refused, `RUN_NOT_MUTABLE` (Decision 044-162) |
| correctness D; architecture 1 (terminate does not exist) | The remedy is cancellation, immediate for a run under review (Decision 044-161) |
| correctness E (`EXCLUDE` makes later points ineligible) | `run.code_version_moved` `COPY`; code `saga.aborting` `COPY` once settled (Decision 044-163) |
| architecture 2 (the move function's last-row check) | Called right after the move event; the replay check row holds run, `S(k)`, chain and verdict (Decision 044-162) |
| architecture 3 (deferrals as gRPC errors) | `ACCEPTED_DEFERRED` outcome of a successful answer (Decision 044-169) |
| architecture 4 (workflow-only bindings) | Stored with its profile; the binding store reads both tables (Decision 044-170) |
| architecture 7 (source checks never run on pull requests) | Hosted as tests of `apps/execution-engine`, which scoped CI selects for any scanned crate (Decision 044-171) |

The 5.0.0 draft's move clause (architecture Nit 14) is registered as Gap 044-025, beside Gaps 044-021 and 044-024, for
the draft's owner.
